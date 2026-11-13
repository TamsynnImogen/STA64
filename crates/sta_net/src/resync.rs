//! Snapshot transfer between peers (NET-04, NET_DECISIONS N2 to N4): a peer
//! that dropped comes back into a running game ([`Rejoiner`]), and a peer
//! that desynced is repaired from the host's state ([`Peer`] does it itself
//! while its match is stopped). The two paths share the transfer and nothing
//! else: a rejoin starts from nothing but a connection, a resync from a live
//! router that is stopped.
//!
//! # What travels
//!
//! `[Start]` (rejoin only), [`SnapshotOffer`] (digest, section digests, size,
//! the router state of the host at that turn: [`RouterSeed`]), numbered
//! [`SnapshotPart`]s, the turn records every seat has queued from that turn on
//! as `Replayed`, then `CatchUpDone`. Everything is directed host to peer and
//! paced (the relay's byte rate is 128 KiB/s). The peer loads the snapshot,
//! checks the digest and every section digest, adopts the router state and
//! replays the records through its normal `step`.
//!
//! # Trust model
//!
//! The host is the authority (N3, and N5 until a referee exists): the state
//! it sends is the state the game continues from, and a peer cannot check
//! it against a second source at the moment it loads it, because the other
//! peers are stalled or send nothing the host did not also see. What a peer
//! does check, before and after applying anything:
//!
//! - it asked: unsolicited snapshot traffic is dropped, not buffered;
//! - sender, format, content hash, the snapshot's own provenance against the
//!   host's seat in `Start`, and sizes, all before the first byte is buffered
//!   for a part, with fixed bounds on size, parts, records and time;
//! - the FNV-1a hash of the bytes (damage, not authentication), the
//!   snapshot's own header, a successful load, and the host's digest and
//!   section digests against the loaded sim, naming the differing sections;
//! - every catch-up record's authority, as for a live record, and that the
//!   seat set, record order and counts fit the seed;
//! - afterwards, every live record another peer sends that repeats one the
//!   host carried must be identical to it (`RouterError::Contradiction`), and
//!   every digest in later records is compared with the loaded sim's as
//!   usual, so a host that lies about the state is found by the first peer
//!   that disagrees (and then the match stops, N4.4).
//!
//! A host that hands one peer a state the others do not hold therefore
//! cannot hide it, but it can still choose which state the game continues
//! from; that is what N3 accepts.
//!
//! # Host limits
//!
//! At most [`ResyncLimits::max_jobs`] transfers at once (a snapshot is held
//! once per boundary, shared); per seat a fixed ledger (no growth with
//! attempts): rejoins, resyncs, bad attempts, and the time of the last
//! attempt; requests inside `min_attempt_interval` are ignored without a
//! reply; a seat over a limit is refused and reported
//! ([`ResyncEvent::LimitReached`], the stall dialog's cue); a seat with too
//! many bad attempts is reported ([`ResyncEvent::SeatAbuse`]) for the
//! lobby to kick.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, Tick};
use sta_sim::digest::SectionDigests;
use sta_sim::setup::GameSetup;
use sta_sim::snapshot::{snapshot_info, SnapshotError, FORMAT_VERSION};
use sta_sim::{Sim, WorldSource};

use crate::lockstep::{Destination, Incoming, Peer, StepOutcome, Transport, TransportError};
use crate::proto::{
    Arrived, EngineId, Hello, Message, ProtoError, Refusal, Rejoin, ResyncAck, ResyncRequest,
    Resynced, SnapshotKind, SnapshotOffer, SnapshotPart, SnapshotRefusal, Start,
    TurnCommands, MAX_SEATS, MAX_SECTIONS, MAX_SNAPSHOT_PART_BYTES,
};

/// FNV-1a 64 over `bytes`: the damage check on a transfer.
pub fn snapshot_hash(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Limits and timing of both sides. [`ResyncLimits::default`] is the
/// production setting; tests shorten the times.
#[derive(Debug, Clone)]
pub struct ResyncLimits {
    /// Times a seat may be resynced in one match (N4.5); then it is a
    /// stalled seat.
    pub resyncs_per_seat: u8,
    /// Times a seat may be taken back in one match.
    pub rejoins_per_seat: u8,
    /// Bad rejoin attempts (wrong token, not vacated, wrong build) before a
    /// seat is reported as abusive.
    pub bad_attempts: u8,
    /// Requests from one seat closer together than this are ignored.
    pub min_attempt_interval: Duration,
    /// Transfers running at once.
    pub max_jobs: usize,
    /// Bytes per second the host sends a transfer at (the relay's limit is
    /// 128 KiB/s per connection, and the lockstep stream shares it).
    pub bytes_per_sec: u32,
    /// Bytes a transfer may save up.
    pub burst_bytes: u32,
    /// A transfer that has not been fully sent after this long is dropped.
    pub send_deadline: Duration,
    /// How long the host waits for the ack after sending everything.
    pub ack_wait: Duration,
    /// How long a request may wait for a usable turn boundary.
    pub boundary_wait: Duration,
    /// After the first resync request, how long the host waits for the other
    /// diverged seats to ask before it releases the stopped peers.
    pub group_window: Duration,
    /// Receiver: how long to wait for the offer (and, for a rejoin, the
    /// `Rejoined` and `Start`).
    pub offer_wait: Duration,
    /// Receiver: how long the transfer may go without a message.
    pub idle_wait: Duration,
    /// Receiver: how long the whole transfer may take.
    pub total_wait: Duration,
    /// Receiver (not diverged): how long to wait for the host to say the
    /// desync was repaired before telling the player the host is stalled.
    pub repair_wait: Duration,
}

impl Default for ResyncLimits {
    fn default() -> ResyncLimits {
        ResyncLimits {
            resyncs_per_seat: 3,
            rejoins_per_seat: 8,
            bad_attempts: 5,
            min_attempt_interval: Duration::from_secs(2),
            max_jobs: 2,
            bytes_per_sec: 96 * 1024,
            burst_bytes: 128 * 1024,
            send_deadline: Duration::from_secs(150),
            ack_wait: Duration::from_secs(30),
            boundary_wait: Duration::from_secs(15),
            group_window: Duration::from_secs(20),
            offer_wait: Duration::from_secs(20),
            idle_wait: Duration::from_secs(15),
            total_wait: Duration::from_secs(180),
            repair_wait: Duration::from_secs(60),
        }
    }
}

/// Why a catch-up failed on the receiving side. Nothing was applied.
#[derive(Debug, Clone, PartialEq)]
pub enum CatchUpError {
    /// The host declined.
    Refused(SnapshotRefusal),
    /// The host's `Refuse` (a rejoin).
    RefusedRejoin(Refusal),
    /// A step did not happen in time (named).
    Timeout(&'static str),
    /// The host sent something out of order, out of bounds or not allowed.
    Protocol(&'static str),
    /// A record in the catch-up broke the authority rules.
    Authority(ProtoError),
    /// The bytes did not match the offer's hash or size.
    Damaged(&'static str),
    /// Another snapshot format than this build reads.
    Format { theirs: u16, ours: u16 },
    /// Taken with another content database than this peer's.
    Content { theirs: u64, ours: u64 },
    /// The snapshot is not this session's host's (its provenance differs
    /// from the host's seat in `Start`).
    Provenance,
    /// The snapshot did not load.
    Snapshot(SnapshotError),
    /// The loaded sim does not match the offered digest; the differing
    /// sections are named.
    Digest {
        theirs: u64,
        ours: u64,
        sections: Vec<&'static str>,
    },
    /// The router could not take the host's state (see its error).
    Router(String),
    /// The transport failed.
    Transport(TransportError),
}

impl std::fmt::Display for CatchUpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatchUpError::Refused(w) => write!(f, "the host declined: {w:?}"),
            CatchUpError::RefusedRejoin(w) => write!(f, "the host refused the rejoin: {w:?}"),
            CatchUpError::Timeout(w) => write!(f, "timed out waiting for {w}"),
            CatchUpError::Protocol(w) => write!(f, "bad catch-up: {w}"),
            CatchUpError::Authority(e) => write!(f, "catch-up record not allowed: {e}"),
            CatchUpError::Damaged(w) => write!(f, "snapshot damaged: {w}"),
            CatchUpError::Format { theirs, ours } => {
                write!(f, "snapshot format {theirs}, this build reads {ours}")
            }
            CatchUpError::Content { theirs, ours } => write!(
                f,
                "snapshot content hash {theirs:#018x}, this peer's is {ours:#018x}"
            ),
            CatchUpError::Provenance => write!(f, "snapshot is not from this game's host"),
            CatchUpError::Snapshot(e) => write!(f, "{e}"),
            CatchUpError::Digest {
                theirs,
                ours,
                sections,
            } => write!(
                f,
                "loaded state digest {ours:#018x}, host's {theirs:#018x}; differing sections: {}",
                if sections.is_empty() { "none named".to_owned() } else { sections.join(", ") }
            ),
            CatchUpError::Router(w) => write!(f, "router could not take the host's state: {w}"),
            CatchUpError::Transport(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CatchUpError {}

/// What the layers above (the lobby, the stall dialog, the sidecar) learn
/// from this module. Read with [`Peer::take_resync_events`].
#[derive(Debug, Clone, PartialEq)]
pub enum ResyncEvent {
    /// Host: a seat's rejoin was accepted; the transfer follows.
    RejoinAccepted { slot: PlayerId },
    /// Host: a rejoin was refused, and why.
    RejoinRefused { slot: PlayerId, why: &'static str },
    /// Host: a seat asked too often or too wrongly. The lobby may kick it.
    SeatAbuse { slot: PlayerId },
    /// Host: a seat used all its rejoins or resyncs: it is a stalled seat
    /// (NET.md section 8); the stall dialog decides.
    LimitReached { slot: PlayerId, kind: SnapshotKind },
    /// Host: a transfer was fully sent.
    Sent { slot: PlayerId, kind: SnapshotKind, bytes: usize },
    /// Host: a transfer ended without the ack.
    TransferFailed { slot: PlayerId, why: &'static str },
    /// Host: a peer loaded a snapshot whose digest is not the host's.
    AckMismatch { slot: PlayerId, theirs: u64, ours: u64 },
    /// A seat is back (rejoin). Every peer gets this.
    Arrived { slot: PlayerId, turn: u32 },
    /// A seat was resynced (N3: every player is told). `last`: the stopped
    /// peers play on.
    Resynced { slot: PlayerId, turn: u32, count: u8, last: bool },
    /// This peer asked for a resync.
    ResyncRequested { tick: Tick },
    /// This peer loaded the host's state and plays on.
    Repaired { turn: u32 },
    /// This peer's catch-up failed; the match stays stopped (N4.4).
    Failed(CatchUpError),
    /// Referee: a seat asked for the state and the policy
    /// (`RefereePolicy::resync_host`) does not resync the host: the match
    /// ends with the reports.
    HostEndsMatch { slot: PlayerId },
    /// This peer is not diverged but the host has not said the desync was
    /// repaired in [`ResyncLimits::repair_wait`]: the stall dialog's cue.
    RepairOverdue,
}

/// A resync this peer went through, for its sidecar and replay note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResyncNote {
    /// The first tick at which this peer's digest differed from the host's.
    pub first_differing: Tick,
    /// This peer's digest there.
    pub had: u64,
    /// The tick the host's state was loaded at.
    pub tick: Tick,
    pub turn: u32,
}

// ---------------------------------------------------------------------------
// Receiving: the shared assembler
// ---------------------------------------------------------------------------

/// What a receiver knows before anything arrives.
#[derive(Clone)]
pub(crate) struct Expect {
    pub kind: SnapshotKind,
    pub host: PlayerId,
    /// Who sends the transfer: the host, or the referee when there is one
    /// (NET-05). Anything else is not the transfer's.
    pub source: PlayerId,
    /// The seats that send records.
    pub seats: Vec<PlayerId>,
    pub content_hash: u64,
    /// The host seat's lineage and pack names in `Start`; `None` from a
    /// referee, whose lineage and packs are its own (D10: informational;
    /// the content hash and the digests are what must agree).
    pub provenance: Option<(String, Vec<String>)>,
}

/// What a finished catch-up delivers, structurally checked.
pub(crate) struct Delivered {
    pub offer: SnapshotOffer,
    pub bytes: Vec<u8>,
    pub records: Vec<TurnCommands>,
}

/// Assembles one catch-up from the messages the host sends, enforcing order,
/// bounds and time. Memory grows with what has arrived, never with what the
/// offer claims.
pub(crate) struct CatchUp {
    expect: Expect,
    limits: ResyncLimits,
    started: Instant,
    last: Instant,
    offer: Option<SnapshotOffer>,
    bytes: Vec<u8>,
    next_part: u16,
    verified: bool,
    records: Vec<TurnCommands>,
    /// Per seat in seed order: how many records arrived.
    seat_at: usize,
    seat_got: u16,
    record_bytes: usize,
}

/// Wire bytes of catch-up records a receiver accepts in all: far more than
/// any real game's queued turns (a few dozen bytes each), far less than 1024
/// maximal frames.
pub const MAX_CATCH_UP_RECORD_BYTES: usize = 8 * 1024 * 1024;

impl CatchUp {
    pub(crate) fn new(expect: Expect, limits: ResyncLimits, now: Instant) -> CatchUp {
        CatchUp {
            expect,
            limits,
            started: now,
            last: now,
            offer: None,
            bytes: Vec::new(),
            next_part: 0,
            verified: false,
            records: Vec::new(),
            seat_at: 0,
            seat_got: 0,
            record_bytes: 0,
        }
    }

    pub(crate) fn source(&self) -> PlayerId {
        self.expect.source
    }

    /// Deadlines.
    pub(crate) fn check_time(&self, now: Instant) -> Result<(), CatchUpError> {
        let since = |t: Instant| now.saturating_duration_since(t);
        if self.offer.is_none() && since(self.started) > self.limits.offer_wait {
            return Err(CatchUpError::Timeout("the snapshot offer"));
        }
        if since(self.last) > self.limits.idle_wait {
            return Err(CatchUpError::Timeout("the next part of the snapshot"));
        }
        if since(self.started) > self.limits.total_wait {
            return Err(CatchUpError::Timeout("the transfer to finish"));
        }
        Ok(())
    }

    /// Takes one message from the host. `Ok(Some(_))` once `CatchUpDone`
    /// arrived and everything checked.
    pub(crate) fn feed(
        &mut self,
        from: PlayerId,
        msg: Message,
        now: Instant,
    ) -> Result<Option<Delivered>, CatchUpError> {
        if from != self.expect.source {
            return Err(CatchUpError::Protocol("snapshot traffic from the wrong sender"));
        }
        self.last = now;
        match msg {
            Message::SnapshotRefused(why) => Err(CatchUpError::Refused(why)),
            Message::SnapshotOffer(o) => {
                if self.offer.is_some() {
                    return Err(CatchUpError::Protocol("a second offer"));
                }
                self.accept_offer(&o)?;
                self.offer = Some(o);
                Ok(None)
            }
            Message::SnapshotPart(p) => self.accept_part(p).map(|()| None),
            Message::Replayed(r) => self.accept_record(r).map(|()| None),
            Message::CatchUpDone => self.finish().map(Some),
            _ => Err(CatchUpError::Protocol("unexpected message in a catch-up")),
        }
    }

    fn accept_offer(&self, o: &SnapshotOffer) -> Result<(), CatchUpError> {
        if o.kind != self.expect.kind {
            return Err(CatchUpError::Protocol("offer of another kind"));
        }
        if o.format != FORMAT_VERSION {
            return Err(CatchUpError::Format {
                theirs: o.format,
                ours: FORMAT_VERSION,
            });
        }
        if o.content_hash != self.expect.content_hash {
            return Err(CatchUpError::Content {
                theirs: o.content_hash,
                ours: self.expect.content_hash,
            });
        }
        let tpt = Tick::from(o.seed.cfg.ticks_per_turn.max(1));
        if o.tick != Tick::from(o.seed.turn) * tpt {
            return Err(CatchUpError::Protocol("snapshot is not at a turn boundary"));
        }
        let slots: Vec<PlayerId> = o.seed.seats.iter().map(|s| s.slot).collect();
        if slots != self.expect.seats {
            return Err(CatchUpError::Protocol("seat set differs from the game's"));
        }
        if o.sections.is_empty() || o.sections.len() > MAX_SECTIONS {
            return Err(CatchUpError::Protocol("section digests"));
        }
        if o.seed.seats.iter().any(|s| s.next_turn < u32::from(s.queued)) {
            return Err(CatchUpError::Protocol("seat's queue longer than its stream"));
        }
        Ok(())
    }

    fn accept_part(&mut self, p: SnapshotPart) -> Result<(), CatchUpError> {
        let Some(o) = &self.offer else {
            return Err(CatchUpError::Protocol("part before the offer"));
        };
        if p.index != self.next_part || self.next_part >= o.parts {
            return Err(CatchUpError::Protocol("part out of order"));
        }
        let (len, chunk, parts) = (o.len as usize, o.chunk as usize, usize::from(o.parts));
        let want = if usize::from(p.index) + 1 == parts {
            len - chunk * (parts - 1)
        } else {
            chunk
        };
        if p.data.len() != want || self.bytes.len() + p.data.len() > len {
            return Err(CatchUpError::Damaged("part size"));
        }
        self.bytes.extend_from_slice(&p.data);
        self.next_part += 1;
        if usize::from(self.next_part) == parts {
            if snapshot_hash(&self.bytes) != o.hash {
                return Err(CatchUpError::Damaged("hash"));
            }
            let info = snapshot_info(&self.bytes).map_err(CatchUpError::Snapshot)?;
            if info.content_hash != o.content_hash {
                return Err(CatchUpError::Damaged("content hash differs from the offer's"));
            }
            if let Some((lineage, packs)) = &self.expect.provenance
                && (&info.provenance.lineage != lineage || &info.provenance.packs != packs)
            {
                return Err(CatchUpError::Provenance);
            }
            self.verified = true;
        }
        Ok(())
    }

    fn accept_record(&mut self, r: TurnCommands) -> Result<(), CatchUpError> {
        let Some(o) = &self.offer else {
            return Err(CatchUpError::Protocol("record before the offer"));
        };
        if !self.verified {
            return Err(CatchUpError::Protocol("record before the snapshot is complete"));
        }
        let seats = &o.seed.seats;
        while seats
            .get(self.seat_at)
            .is_some_and(|s| self.seat_got >= s.queued)
        {
            self.seat_at += 1;
            self.seat_got = 0;
        }
        let Some(seat) = seats.get(self.seat_at) else {
            return Err(CatchUpError::Protocol("more records than the seed announced"));
        };
        // A seat's records end at its next turn and run contiguously back.
        let first = seat.next_turn - u32::from(seat.queued);
        if r.slot != seat.slot || r.turn != first + u32::from(self.seat_got) {
            return Err(CatchUpError::Protocol("record out of order"));
        }
        let host = self.expect.host;
        let speaks = |p: PlayerId| p == PlayerId::SCRIPT || o.seed.host_speaks.contains(&p);
        r.authorize(r.slot, r.slot == host, speaks)
            .map_err(CatchUpError::Authority)?;
        self.record_bytes += postcard::to_allocvec(&r).map_or(usize::MAX / 2, |b| b.len());
        if self.record_bytes > MAX_CATCH_UP_RECORD_BYTES {
            return Err(CatchUpError::Protocol("catch-up records over the byte bound"));
        }
        self.seat_got += 1;
        self.records.push(r);
        Ok(())
    }

    fn finish(&mut self) -> Result<Delivered, CatchUpError> {
        let Some(o) = self.offer.take() else {
            return Err(CatchUpError::Protocol("done before the offer"));
        };
        let total: usize = o.seed.seats.iter().map(|s| usize::from(s.queued)).sum();
        if !self.verified || self.records.len() != total {
            return Err(CatchUpError::Protocol("catch-up incomplete"));
        }
        Ok(Delivered {
            offer: o,
            bytes: std::mem::take(&mut self.bytes),
            records: std::mem::take(&mut self.records),
        })
    }
}

/// Loads a delivered snapshot and checks it against the offer: format and
/// content were checked on arrival, here the sim must load, sit at the
/// offered tick, and match the full digest and every section digest. A
/// mismatch names the sections that differ.
pub(crate) fn load(d: &Delivered, content: &Arc<ContentDb>) -> Result<Sim, CatchUpError> {
    let sim = Sim::from_snapshot(&d.bytes, Some(Arc::clone(content))).map_err(CatchUpError::Snapshot)?;
    if sim.view().tick != d.offer.tick {
        return Err(CatchUpError::Protocol("snapshot tick differs from the offer's"));
    }
    let digest = sim.digest();
    let ours = sim.section_digests();
    let theirs = &d.offer.sections;
    let differing: Vec<&'static str> = SectionDigests::NAMES
        .iter()
        .enumerate()
        .filter(|(i, _)| theirs.get(*i) != ours.values().get(*i))
        .map(|(_, n)| *n)
        .collect();
    if digest != d.offer.digest || !differing.is_empty() || theirs.len() != ours.values().len() {
        return Err(CatchUpError::Digest {
            theirs: d.offer.digest,
            ours: digest,
            sections: differing,
        });
    }
    Ok(sim)
}

// ---------------------------------------------------------------------------
// Rejoining: the peer that was gone
// ---------------------------------------------------------------------------

/// Why a rejoin failed. The transport is closed; try again on a new
/// connection (the host rate-limits attempts).
#[derive(Debug)]
pub enum RejoinError {
    CatchUp(CatchUpError),
    /// The host's `Start` is not for this peer's seat, rules or build.
    Start(&'static str),
    /// Too much live traffic arrived while catching up.
    Overflow,
    /// The peer could not be built from what arrived.
    Build(ProtoError),
}

impl std::fmt::Display for RejoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejoinError::CatchUp(e) => write!(f, "{e}"),
            RejoinError::Start(w) => write!(f, "bad start for a rejoin: {w}"),
            RejoinError::Overflow => write!(f, "too much live traffic during the catch-up"),
            RejoinError::Build(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RejoinError {}

impl From<CatchUpError> for RejoinError {
    fn from(e: CatchUpError) -> RejoinError {
        RejoinError::CatchUp(e)
    }
}

/// A finished rejoin.
pub struct RejoinDone {
    /// The peer, at the snapshot's tick, with the others' queued records;
    /// step it like any peer. It has already told the host its digest.
    pub peer: Peer,
    /// The token for the next rejoin (the old one is spent).
    pub token: u64,
}

/// What [`Rejoiner::poll`] found.
pub enum RejoinProgress {
    Waiting,
    Done(Box<RejoinDone>),
}

enum RejoinState {
    /// `Rejoin` sent; waiting for `Rejoined`.
    Reply,
    /// `Rejoined` came; waiting for `Start`.
    Start { token: u64, last_turn: Option<u32> },
    /// Receiving the snapshot.
    Catching {
        token: u64,
        last_turn: Option<u32>,
        start: Box<Start>,
        catch: Box<CatchUp>,
    },
}

/// The peer side of a rejoin (NET.md section 8, N4): holds the new
/// connection, sends `Rejoin`, takes `Rejoined`, `Start` and the snapshot,
/// and ends in a [`Peer`] that resumes in lockstep. Poll it from the game
/// loop; it never blocks. Messages other peers send meanwhile are kept (at
/// most [`Rejoiner::MAX_LIVE`]) and handed to the router afterwards.
pub struct Rejoiner {
    transport: Option<Box<dyn Transport>>,
    me: PlayerId,
    host: PlayerId,
    content: Arc<ContentDb>,
    map: Arc<MapDef>,
    hello: Hello,
    limits: ResyncLimits,
    started: Instant,
    state: RejoinState,
    live: Vec<Incoming>,
    live_bytes: usize,
    new_token: Option<u64>,
}

impl Rejoiner {
    /// Live messages kept while catching up.
    pub const MAX_LIVE: usize = 4096;
    /// Bytes of them (at their wire size).
    pub const MAX_LIVE_BYTES: usize = 8 * 1024 * 1024;

    /// Sends `Rejoin` for seat `me` (the one the transport reclaimed) with
    /// the token from `Welcome` or the last `Rejoined`, and the same
    /// `Hello` a joiner shows. `content` and `map` are this peer's own.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        mut transport: Box<dyn Transport>,
        me: PlayerId,
        host: PlayerId,
        hello: Hello,
        token: u64,
        content: Arc<ContentDb>,
        map: Arc<MapDef>,
        limits: ResyncLimits,
        now: Instant,
    ) -> Result<Rejoiner, TransportError> {
        transport.send(
            Destination::Seat(host),
            &Message::Rejoin(Rejoin {
                hello: hello.clone(),
                slot: me,
                token,
                next_turn: None,
            }),
        )?;
        Ok(Rejoiner {
            transport: Some(transport),
            me,
            host,
            content,
            map,
            hello,
            limits,
            started: now,
            state: RejoinState::Reply,
            live: Vec::new(),
            live_bytes: 0,
            new_token: None,
        })
    }

    /// The token to rejoin with next, once the host has answered: the old one
    /// is spent at that point, so keep this when an attempt fails after it.
    pub fn current_token(&self) -> Option<u64> {
        self.new_token
    }

    pub fn poll(&mut self) -> Result<RejoinProgress, RejoinError> {
        self.poll_at(Instant::now())
    }

    /// [`Rejoiner::poll`] at a given time (tests).
    pub fn poll_at(&mut self, now: Instant) -> Result<RejoinProgress, RejoinError> {
        loop {
            let Some(t) = self.transport.as_mut() else {
                return Err(RejoinError::CatchUp(CatchUpError::Protocol("rejoin already ended")));
            };
            match t.poll() {
                Ok(Some(Incoming { from, msg })) => match self.handle(from, msg, now) {
                    Ok(Some(done)) => return Ok(RejoinProgress::Done(done)),
                    Ok(None) => {}
                    Err(e) => {
                        self.transport = None;
                        return Err(e);
                    }
                },
                Ok(None) => break,
                // A frame that does not decode is the sender's fault, as in
                // the router: drop it and go on.
                Err(TransportError::Frame(_)) => {}
                Err(e) => {
                    self.transport = None;
                    return Err(RejoinError::CatchUp(CatchUpError::Transport(e)));
                }
            }
        }
        let late = match &self.state {
            RejoinState::Reply | RejoinState::Start { .. } => {
                now.saturating_duration_since(self.started) > self.limits.offer_wait
                    .max(Duration::from_millis(1))
            }
            RejoinState::Catching { catch, .. } => catch.check_time(now).is_err(),
        };
        if late {
            let what = match &self.state {
                RejoinState::Reply => "the host's answer",
                RejoinState::Start { .. } => "the game's start",
                RejoinState::Catching { catch, .. } => {
                    catch.check_time(now).err().map_or("the snapshot", |e| match e {
                        CatchUpError::Timeout(w) => w,
                        _ => "the snapshot",
                    })
                }
            };
            self.transport = None;
            return Err(RejoinError::CatchUp(CatchUpError::Timeout(what)));
        }
        Ok(RejoinProgress::Waiting)
    }

    fn handle(
        &mut self,
        from: PlayerId,
        msg: Message,
        now: Instant,
    ) -> Result<Option<Box<RejoinDone>>, RejoinError> {
        if msg.check_sender(from == self.host).is_err() {
            return Ok(None);
        }
        // Live lockstep traffic from anyone, the host included: kept for the
        // router. (A `Resume` may be counted twice, once here and once in the
        // seed; an extra resume only lets this peer pass a pause early, which
        // changes no sim state, where a missing one would stall it.)
        if matches!(
            msg,
            Message::Turn(_)
                | Message::Desync(_)
                | Message::Sections(_)
                | Message::Leave(_)
                | Message::Resume { .. }
                | Message::RefereeSeated { .. }
        ) {
            let size = postcard::to_allocvec(&msg).map_or(usize::MAX / 2, |b| b.len() + 16);
            if self.live.len() >= Self::MAX_LIVE || self.live_bytes + size > Self::MAX_LIVE_BYTES {
                return Err(RejoinError::Overflow);
            }
            self.live_bytes += size;
            self.live.push(Incoming { from, msg });
            return Ok(None);
        }
        // Chat and pings are not worth keeping, and a broadcast about
        // another seat (`Arrived`, `Resynced`) is not this transfer's.
        if from != self.host
            || matches!(
                msg,
                Message::Chat(_)
                    | Message::Ping { .. }
                    | Message::Pong { .. }
                    | Message::Arrived(_)
                    | Message::Resynced(_)
            )
        {
            return Ok(None);
        }
        match (&mut self.state, msg) {
            (_, Message::Refuse(why)) => {
                Err(RejoinError::CatchUp(CatchUpError::RefusedRejoin(why)))
            }
            (_, Message::SnapshotRefused(why)) => Err(RejoinError::CatchUp(CatchUpError::Refused(why))),
            (RejoinState::Reply, Message::Rejoined(r)) => {
                self.new_token = Some(r.rejoin_token);
                self.state = RejoinState::Start {
                    token: r.rejoin_token,
                    last_turn: r.last_turn,
                };
                Ok(None)
            }
            (RejoinState::Start { token, last_turn }, Message::Start(start)) => {
                let (token, last_turn) = (*token, *last_turn);
                self.check_start(&start)?;
                let host_seat = start
                    .seats
                    .iter()
                    .find(|s| s.slot == start.host)
                    .ok_or(RejoinError::Start("host is not a seat"))?;
                let expect = Expect {
                    kind: SnapshotKind::Rejoin,
                    host: self.host,
                    source: self.host,
                    seats: start
                        .seats
                        .iter()
                        .filter(|s| s.occupant == crate::proto::Occupant::Peer)
                        .map(|s| s.slot)
                        .collect(),
                    content_hash: self.content.hash(),
                    provenance: Some((
                        host_seat.lineage.clone(),
                        host_seat.packs.iter().map(|p| p.name.clone()).collect(),
                    )),
                };
                let catch = Box::new(CatchUp::new(expect, self.limits.clone(), now));
                self.state = RejoinState::Catching {
                    token,
                    last_turn,
                    start: Box::new(start),
                    catch,
                };
                Ok(None)
            }
            (RejoinState::Catching { catch, .. }, m) => match catch.feed(from, m, now)? {
                None => Ok(None),
                Some(d) => self.finish(d).map(Some),
            },
            _ => Err(RejoinError::CatchUp(CatchUpError::Protocol(
                "message out of order in a rejoin",
            ))),
        }
    }

    fn check_start(&self, start: &Start) -> Result<(), RejoinError> {
        if start.host != self.host {
            return Err(RejoinError::Start("host seat differs"));
        }
        if start.rules != self.hello.rules {
            return Err(RejoinError::Start("rules digest differs from this peer's"));
        }
        if !start
            .seats
            .iter()
            .any(|s| s.slot == self.me && s.occupant == crate::proto::Occupant::Peer)
        {
            return Err(RejoinError::Start("this seat is not a person's seat"));
        }
        Ok(())
    }

    fn finish(&mut self, d: Delivered) -> Result<Box<RejoinDone>, RejoinError> {
        let RejoinState::Catching { token, last_turn, start, .. } =
            std::mem::replace(&mut self.state, RejoinState::Reply)
        else {
            return Err(RejoinError::CatchUp(CatchUpError::Protocol("not catching up")));
        };
        let sim = load(&d, &self.content)?;
        let own_next = d
            .offer
            .seed
            .seats
            .iter()
            .find(|s| s.slot == self.me)
            .map(|s| s.next_turn);
        // The host's account of our last record must be the seed's.
        if let (Some(l), Some(n)) = (last_turn, own_next)
            && l.checked_add(1) != Some(n)
        {
            return Err(RejoinError::CatchUp(CatchUpError::Protocol(
                "last record differs from the seed's",
            )));
        }
        let transport = self.transport.take().ok_or(RejoinError::Overflow)?;
        let peer = Peer::from_catch_up(
            &start,
            self.me,
            Arc::clone(&self.content),
            Arc::clone(&self.map),
            transport,
            &d,
            sim,
            std::mem::take(&mut self.live),
        )
        .map_err(|e| match e {
            CatchUpError::Router(w) => RejoinError::CatchUp(CatchUpError::Router(w)),
            other => RejoinError::CatchUp(other),
        })?;
        Ok(Box::new(RejoinDone { peer, token }))
    }
}

// ---------------------------------------------------------------------------
// Serving: the host
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct Ledger {
    rejoins: u8,
    resyncs: u8,
    bad: u8,
    last_attempt: Option<Instant>,
    abuse_reported: bool,
}

struct Policy {
    engine: EngineId,
    tokens: BTreeMap<PlayerId, u64>,
}

struct Transfer {
    start: Option<Start>,
    offer: SnapshotOffer,
    bytes: Arc<Vec<u8>>,
    records: Vec<TurnCommands>,
    cursor: usize,
    tokens: f64,
    refilled: Instant,
    began: Instant,
}

impl Transfer {
    fn len(&self) -> usize {
        usize::from(self.start.is_some()) + 1 + usize::from(self.offer.parts) + self.records.len() + 1
    }

    /// The `i`th message and its cost in bytes.
    fn message(&self, i: usize) -> (Message, usize) {
        let mut i = i;
        if let Some(s) = &self.start {
            if i == 0 {
                return (Message::Start(s.clone()), 512);
            }
            i -= 1;
        }
        if i == 0 {
            return (Message::SnapshotOffer(self.offer.clone()), 512);
        }
        i -= 1;
        let parts = usize::from(self.offer.parts);
        if i < parts {
            let chunk = self.offer.chunk as usize;
            let from = i * chunk;
            let to = (from + chunk).min(self.bytes.len());
            let data = self.bytes[from..to].to_vec();
            let cost = data.len() + 16;
            return (
                Message::SnapshotPart(SnapshotPart {
                    index: u16::try_from(i).unwrap_or(u16::MAX),
                    data,
                }),
                cost,
            );
        }
        i -= parts;
        if let Some(r) = self.records.get(i) {
            return (Message::Replayed(r.clone()), 256);
        }
        (Message::CatchUpDone, 16)
    }
}

enum Phase {
    /// Waiting for a usable turn boundary.
    Await,
    Send(Box<Transfer>),
    /// Everything sent; waiting for the ack.
    Sent { at: Instant, digest: u64, tick: Tick, turn: u32 },
}

struct Job {
    seat: PlayerId,
    kind: SnapshotKind,
    /// Resync: the requester's next turn.
    position: u32,
    created: Instant,
    phase: Phase,
}

struct Group {
    /// Diverged seats that have not been resynced yet.
    expected: BTreeSet<PlayerId>,
    until: Instant,
    failed: bool,
}

/// Host-side state (see the module docs: limits).
pub(crate) struct HostState {
    limits: ResyncLimits,
    policy: Option<Policy>,
    vacated: BTreeSet<PlayerId>,
    ledger: [Ledger; MAX_SEATS],
    jobs: Vec<Job>,
    group: Option<Group>,
    departures: Option<Box<dyn FnMut() -> Vec<PlayerId> + Send>>,
    /// The snapshot of the current boundary, shared by the jobs that start
    /// there.
    cache: Option<(Tick, Arc<Vec<u8>>)>,
    rng: u64,
}

impl HostState {
    fn ledger(&mut self, seat: PlayerId) -> Option<&mut Ledger> {
        self.ledger.get_mut(usize::from(seat.0))
    }

    fn token(&mut self) -> u64 {
        let mut b = [0u8; 8];
        if getrandom::fill(&mut b).is_ok() {
            return u64::from_le_bytes(b);
        }
        // No OS randomness: a counter mixed from the clock; still never
        // repeats within a match.
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        self.rng ^ (Instant::now().elapsed().as_nanos() as u64)
    }
}

impl Default for HostState {
    fn default() -> HostState {
        HostState {
            limits: ResyncLimits::default(),
            policy: None,
            vacated: BTreeSet::new(),
            ledger: [Ledger::default(); MAX_SEATS],
            jobs: Vec::new(),
            group: None,
            departures: None,
            cache: None,
            rng: 0x5eed_0004,
        }
    }
}

/// Everything NET-04 keeps in a [`Peer`].
#[derive(Default)]
pub(crate) struct NetState {
    pub(crate) host: HostState,
    client: Option<CatchUp>,
    requested: Option<ResyncRequest>,
    failed: bool,
    events: Vec<ResyncEvent>,
    notes: Vec<ResyncNote>,
    /// When this peer first found itself stopped by a desync it is not part
    /// of (waiting for the host to repair the others).
    waiting_since: Option<Instant>,
    overdue_noted: bool,
    /// After a transient refusal (`Busy`, `Timeout`): ask again after this.
    retry_after: Option<Instant>,
    /// NET-05: what a referee does when the host is the diverged one.
    pub(crate) referee_policy: crate::referee::RefereePolicy,
}

impl NetState {
    /// Sidecar lines (docs/NET.md section 7): each resync this peer went
    /// through, and whether a catch-up failed.
    pub(crate) fn sidecar_lines(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .notes
            .iter()
            .map(|n| {
                format!(
                    "resynced from the host: first differing tick {} (this seat's digest {:#018x}), state loaded at tick {} (turn {}); this replay's digests before that tick may differ from the authority's",
                    n.first_differing, n.had, n.tick, n.turn
                )
            })
            .collect();
        if self.failed {
            v.push("resync failed: the host's state could not be applied; the match ended".to_owned());
        }
        v
    }

    fn note(&mut self, e: ResyncEvent) {
        // Bounded: a hostile peer cannot grow the list.
        if self.events.len() < 256 {
            self.events.push(e);
        }
    }
}

impl Peer {
    /// Host: serve rejoins. `engine` is the host's own build (a rejoiner
    /// must match it exactly, as a joiner does), `tokens` the `rejoin_token`
    /// each seat got in `Welcome`. Without it every `Rejoin` is refused.
    /// Resync requests need no setup.
    pub fn serve_rejoins(&mut self, engine: EngineId, tokens: impl IntoIterator<Item = (PlayerId, u64)>) {
        self.net.host.policy = Some(Policy {
            engine,
            tokens: tokens.into_iter().collect(),
        });
    }

    /// Whether this peer serves resyncs: the referee, and the host while
    /// there is no referee (N3, N5).
    pub(crate) fn serves(&self) -> bool {
        self.router.is_spectator() || (self.router.is_host() && self.router.host_is_authority())
    }

    /// Referee: what to do when the host is the one that differs (see
    /// [`crate::referee::RefereePolicy`]).
    pub fn set_referee_policy(&mut self, policy: crate::referee::RefereePolicy) {
        self.net.referee_policy = policy;
    }

    /// Replaces the default [`ResyncLimits`].
    pub fn set_resync_limits(&mut self, limits: ResyncLimits) {
        self.net.host.limits = limits;
    }

    /// Host: `slot`'s connection ended and its departure marker was read
    /// (`NetControl::take_departures`): it may rejoin. Ends a transfer in
    /// progress to it.
    pub fn note_departed(&mut self, slot: PlayerId) {
        // The referee's connection ended: the host unseats it, so whoever
        // reclaims the relay slot is not believed (NET-05).
        if self.router.is_host() && self.router.referee() == Some(slot) {
            let _ = self.router.seat_referee(None);
        }
        self.net.host.vacated.insert(slot);
        self.net.host.jobs.retain(|j| j.seat != slot);
    }

    /// Host: where departure markers come from, polled every step before
    /// rejoin requests are looked at (so a request is never judged before
    /// the marker that precedes it in the stream): typically
    /// `move || control.take_departures()`.
    pub fn set_departure_source(&mut self, source: impl FnMut() -> Vec<PlayerId> + Send + 'static) {
        self.net.host.departures = Some(Box::new(source));
    }

    /// Host: whether a transfer to `slot` is in progress. The host's own
    /// stall logic must not drop a seat that is on its way back.
    pub fn rejoin_in_progress(&self, slot: PlayerId) -> bool {
        self.net.host.jobs.iter().any(|j| j.seat == slot && j.kind == SnapshotKind::Rejoin)
    }

    /// Takes the events since the last call.
    pub fn take_resync_events(&mut self) -> Vec<ResyncEvent> {
        std::mem::take(&mut self.net.events)
    }

    /// The resyncs this peer went through, oldest first.
    pub fn resyncs(&self) -> &[ResyncNote] {
        &self.net.notes
    }

    /// Whether this peer's own catch-up failed: the match is over for it.
    pub fn resync_failed(&self) -> bool {
        self.net.failed
    }

    /// Whether this peer is waiting for the host's snapshot.
    pub fn resyncing(&self) -> bool {
        self.net.client.is_some()
    }

    fn send_to(&mut self, to: PlayerId, msg: &Message) -> bool {
        self.router.send_message(Destination::Seat(to), msg).is_ok()
    }

    /// A peer built from a rejoin's delivery: the sim at the snapshot's tick,
    /// the router continuing where the host's was, the host's queued records
    /// in it, then whatever other peers sent live meanwhile.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_catch_up(
        start: &Start,
        me: PlayerId,
        content: Arc<ContentDb>,
        map: Arc<MapDef>,
        transport: Box<dyn Transport>,
        d: &Delivered,
        sim: Sim,
        live: Vec<Incoming>,
    ) -> Result<Peer, CatchUpError> {
        let setup = GameSetup {
            seed: start.seed,
            map,
            content,
            players: start.player_setups(me),
            options: start.options.clone(),
        };
        let mut peer = Peer::with_sim(start, &setup, sim, me, transport)
            .map_err(|e| CatchUpError::Router(e.to_string()))?;
        peer.router
            .adopt(&d.offer.seed, d.records.clone())
            .map_err(|e| CatchUpError::Router(e.to_string()))?;
        peer.tick = d.offer.tick;
        peer.replay_from = d.offer.tick;
        peer.replay.digests = vec![(d.offer.tick, d.offer.digest)];
        peer.replay.end_tick = d.offer.tick;
        for Incoming { from, msg } in live {
            if let Err(error) = peer.router.receive(from, msg) {
                peer.router.note_protocol(from, error);
            }
        }
        peer.send_to(
            start.host,
            &Message::ResyncAck(ResyncAck {
                tick: d.offer.tick,
                digest: d.offer.digest,
            }),
        );
        Ok(peer)
    }

    // ---- per step ---------------------------------------------------------

    /// Everything before the sim step: requests and replies, transfers,
    /// timers. Returns an outcome when it used the step itself.
    pub(crate) fn net_pre(&mut self, now: Instant) -> Option<StepOutcome> {
        if let Some(src) = self.net.host.departures.as_mut() {
            for s in src() {
                self.note_departed(s);
            }
        }
        for Incoming { from, msg } in self.router.take_inbound() {
            self.net_dispatch(from, msg, now);
        }
        self.net_progress_client(now);
        if self.router.is_host() || self.router.is_spectator() {
            self.host_timers(now);
            self.host_serve(now);
            // A stopped host that owes a snapshot runs on to the turn
            // boundary it will take it at.
            let tpt = self.ticks_per_turn.max(1);
            if self.router.halted()
                && !self.net.host.jobs.is_empty()
                && !self.tick.is_multiple_of(tpt)
            {
                return Some(self.run_empty_tick());
            }
        }
        if self.net.client.is_some() || self.net.failed {
            return Some(StepOutcome::Halted);
        }
        None
    }

    fn run_empty_tick(&mut self) -> StepOutcome {
        let before = self.tick;
        let out = self.sim.step(&[]);
        self.tick = out.view.tick;
        self.replay
            .log_step(before, &[], self.tick, out.view.digest, self.ticks_per_turn);
        StepOutcome::Ran {
            tick: self.tick,
            events: out.events,
        }
    }

    fn net_dispatch(&mut self, from: PlayerId, msg: Message, now: Instant) {
        match msg {
            Message::Rejoin(r) => self.on_rejoin(from, r, now),
            Message::ResyncRequest(r) => self.on_resync_request(from, r, now),
            Message::ResyncAck(a) => self.on_ack(from, a),
            // Only the authority says who was resynced (the referee when
            // there is one, N5): a host's word does not release peers that
            // wait for a referee, and a seat's is refused by the router.
            Message::Resynced(_) if from != self.router.authority() => {}
            Message::Resynced(r) => {
                self.net.note(ResyncEvent::Resynced {
                    slot: r.slot,
                    turn: r.turn,
                    count: r.count,
                    last: r.last,
                });
                if r.last && self.net.client.is_none() && self.diverged_from_host().is_none() {
                    // A release with nothing resynced (count 0) loaded no
                    // state: nothing is stale.
                    if r.count == 0 {
                        self.router.release_tick(r.turn);
                    } else {
                        self.router.clear_desync_at(r.turn);
                    }
                    self.net.waiting_since = None;
                }
            }
            Message::Arrived(a) => {
                self.net.note(ResyncEvent::Arrived {
                    slot: a.slot,
                    turn: a.turn,
                });
            }
            m @ (Message::SnapshotOffer(_)
            | Message::SnapshotPart(_)
            | Message::SnapshotRefused(_)
            | Message::Replayed(_)
            | Message::CatchUpDone) => {
                let Some(c) = self.net.client.as_mut() else {
                    return;
                };
                // A stray transfer from anyone but the one asked is not ours.
                if from != c.source() {
                    return;
                }
                match c.feed(from, m, now) {
                    Ok(None) => {}
                    Ok(Some(d)) => self.apply_resync(d),
                    Err(CatchUpError::Refused(
                        SnapshotRefusal::Busy | SnapshotRefusal::Timeout | SnapshotRefusal::RateLimited,
                    )) => {
                        // Transient: the host is serving another seat. Ask
                        // again later; the match stays stopped meanwhile.
                        self.router.expect_catch_up(false);
                        self.net.client = None;
                        self.net.retry_after = Some(now + Duration::from_secs(3));
                    }
                    Err(e) => self.fail_resync(e),
                }
            }
            _ => {}
        }
    }

    // ---- the diverged peer ------------------------------------------------

    /// The first tick at which this peer's digest differs from the
    /// authority's (the host's, or the referee's when there is one), and this
    /// peer's digest there; `None` when it does not differ or the
    /// authority's is not known (yet).
    pub fn diverged_from_host(&self) -> Option<(Tick, u64)> {
        self.diverged_from_authority()
    }

    /// See [`Peer::diverged_from_host`].
    pub fn diverged_from_authority(&self) -> Option<(Tick, u64)> {
        let tick = self
            .router
            .desync()
            .map(|n| n.theirs.tick)
            .or_else(|| self.router.peer_desync().map(|(_, n)| n.ours.tick))?;
        let known = self.router.digests_at(tick);
        let ours = known.iter().find(|(p, _)| *p == self.router.me())?.1;
        let theirs = known.iter().find(|(p, _)| *p == self.router.authority())?.1;
        (ours != theirs).then_some((tick, ours))
    }

    fn net_progress_client(&mut self, now: Instant) {
        // The host asks too when a referee is the authority: its state may
        // be the diverged one (N5).
        if self.router.is_host() && self.router.host_is_authority() || self.router.is_spectator() {
            return;
        }
        if let Some(c) = self.net.client.as_ref()
            && let Err(e) = c.check_time(now)
        {
            return self.fail_resync(e);
        }
        if self.net.retry_after.is_some_and(|t| now < t) {
            return;
        }
        if self.net.client.is_some() || self.net.failed || !self.router.halted() {
            if !self.router.halted() {
                self.net.waiting_since = None;
            }
            return;
        }
        match self.diverged_from_authority() {
            Some((tick, digest)) => {
                let req = ResyncRequest {
                    tick,
                    digest,
                    position: self.router.next_turn(),
                };
                let host = self.router.authority();
                let content_hash = self.content.hash();
                let hs = self.start.seats.iter().find(|s| s.slot == self.start.host);
                let provenance = (host == self.router.host_seat()).then(|| {
                    hs.map_or((String::new(), Vec::new()), |s| {
                        (s.lineage.clone(), s.packs.iter().map(|p| p.name.clone()).collect())
                    })
                });
                let seats: Vec<PlayerId> = self.router.seats();
                self.router.expect_catch_up(true);
                let sent = self.send_to(host, &Message::ResyncRequest(req));
                if !sent {
                    self.router.expect_catch_up(false);
                }
                if sent {
                    self.net.requested = Some(req);
                    self.net.client = Some(CatchUp::new(
                        Expect {
                            kind: SnapshotKind::Resync,
                            host: self.router.host_seat(),
                            source: host,
                            seats,
                            content_hash,
                            provenance,
                        },
                        self.net.host.limits.clone(),
                        now,
                    ));
                    self.net.note(ResyncEvent::ResyncRequested { tick });
                }
            }
            None => {
                // Not diverged (or the host's digest is not here yet): the
                // host repairs the others and says so.
                let since = *self.net.waiting_since.get_or_insert(now);
                if !self.net.overdue_noted
                    && now.saturating_duration_since(since) > self.net.host.limits.repair_wait
                {
                    self.net.overdue_noted = true;
                    self.net.note(ResyncEvent::RepairOverdue);
                }
            }
        }
    }

    fn fail_resync(&mut self, e: CatchUpError) {
        self.router.expect_catch_up(false);
        self.net.client = None;
        self.net.failed = true;
        self.net.note(ResyncEvent::Failed(e));
        // A refusal for the limit is the stall dialog's: nothing else to do
        // here; the match stays stopped and the reports are written.
    }

    fn apply_resync(&mut self, d: Delivered) {
        self.router.expect_catch_up(false);
        let sim = match load(&d, &self.content) {
            Ok(s) => s,
            Err(e) => {
                // N4.4: still not the authority's state: the match ends. Tell
                // the host what we hold.
                if let CatchUpError::Digest { ours, .. } = &e {
                    let host = self.router.authority();
                    let ack = ResyncAck { tick: d.offer.tick, digest: *ours };
                    self.send_to(host, &Message::ResyncAck(ack));
                }
                return self.fail_resync(e);
            }
        };
        let (first, had) = self
            .net
            .requested
            .take()
            .map_or((d.offer.tick, 0), |r| (r.tick, r.digest));
        if let Err(e) = self.router.adopt(&d.offer.seed, d.records.clone()) {
            return self.fail_resync(CatchUpError::Router(e.to_string()));
        }
        self.sim = sim;
        self.tick = d.offer.tick;
        self.digested = None;
        // The replay keeps what this peer applied before the snapshot's tick
        // (the same commands everywhere) and goes on from there; its digests
        // before it may differ from the authority's (N4.6).
        self.replay.commands.retain(|c| c.tick < d.offer.tick);
        self.replay.digests.retain(|(t, _)| *t < d.offer.tick);
        self.replay.digests.push((d.offer.tick, d.offer.digest));
        self.replay.end_tick = d.offer.tick;
        self.net.client = None;
        self.net.waiting_since = None;
        self.net.notes.push(ResyncNote {
            first_differing: first,
            had,
            tick: d.offer.tick,
            turn: d.offer.seed.turn,
        });
        self.net.note(ResyncEvent::Repaired {
            turn: d.offer.seed.turn,
        });
        let host = self.router.authority();
        self.send_to(
            host,
            &Message::ResyncAck(ResyncAck {
                tick: d.offer.tick,
                digest: d.offer.digest,
            }),
        );
    }

    // ---- the host ---------------------------------------------------------

    fn refuse(&mut self, to: PlayerId, why: SnapshotRefusal) {
        self.send_to(to, &Message::SnapshotRefused(why));
    }

    /// Counts an attempt; `false` when it came too soon to be answered.
    fn attempt_allowed(&mut self, seat: PlayerId, now: Instant) -> bool {
        let gap = self.net.host.limits.min_attempt_interval;
        let Some(l) = self.net.host.ledger(seat) else {
            return false;
        };
        let ok = l
            .last_attempt
            .is_none_or(|t| now.saturating_duration_since(t) >= gap);
        if ok {
            l.last_attempt = Some(now);
        }
        ok
    }

    /// A bad rejoin attempt: counted; past the limit the seat is reported
    /// once and no longer answered.
    fn bad_rejoin(&mut self, seat: PlayerId, why: &'static str, reply: Message) {
        let limit = self.net.host.limits.bad_attempts;
        let mut report = false;
        let mut silent = false;
        if let Some(l) = self.net.host.ledger(seat) {
            l.bad = l.bad.saturating_add(1);
            silent = l.bad > limit;
            if l.bad >= limit && !l.abuse_reported {
                l.abuse_reported = true;
                report = true;
            }
        }
        self.net.note(ResyncEvent::RejoinRefused { slot: seat, why });
        if report {
            self.net.note(ResyncEvent::SeatAbuse { slot: seat });
        }
        if !silent {
            self.send_to(seat, &reply);
        }
    }

    fn on_rejoin(&mut self, from: PlayerId, r: Rejoin, now: Instant) {
        if !self.router.is_host() || from == self.router.host_seat() {
            return;
        }
        let silent = self
            .net
            .host
            .ledger
            .get(usize::from(from.0))
            .is_some_and(|l| l.bad > self.net.host.limits.bad_attempts);
        if silent || !self.attempt_allowed(from, now) {
            return;
        }
        let bad = Message::Refuse(Refusal::BadRejoin);
        let Some(policy) = self.net.host.policy.as_ref() else {
            return self.bad_rejoin(from, "this host does not serve rejoins", bad);
        };
        if r.slot != from || !self.router.seats().contains(&from) {
            return self.bad_rejoin(from, "not that seat", bad);
        }
        if !self.net.host.vacated.contains(&from) {
            return self.bad_rejoin(from, "the seat is not vacated", bad);
        }
        if policy.tokens.get(&from) != Some(&r.token) {
            return self.bad_rejoin(from, "wrong token", bad);
        }
        if !policy.engine.matches(&r.hello.engine) {
            let reply = Message::Refuse(Refusal::Engine {
                host: policy.engine.clone(),
            });
            return self.bad_rejoin(from, "another build", reply);
        }
        if r.hello.rules != self.start.rules {
            return self.bad_rejoin(from, "other rules", bad);
        }
        let limits = self.net.host.limits.clone();
        let rejoins = self.net.host.ledger.get(usize::from(from.0)).map_or(0, |l| l.rejoins);
        if rejoins >= limits.rejoins_per_seat {
            self.net.note(ResyncEvent::LimitReached {
                slot: from,
                kind: SnapshotKind::Rejoin,
            });
            return self.refuse(from, SnapshotRefusal::LimitReached);
        }
        if self.router.halted() || self.net.host.jobs.len() >= limits.max_jobs {
            return self.refuse(from, SnapshotRefusal::Busy);
        }
        if self.net.host.jobs.iter().any(|j| j.seat == from) {
            return;
        }
        let new_token = self.net.host.token();
        if let Some(p) = self.net.host.policy.as_mut() {
            p.tokens.insert(from, new_token);
        }
        if let Some(l) = self.net.host.ledger(from) {
            l.rejoins += 1;
        }
        self.net.host.vacated.remove(&from);
        self.net.host.jobs.push(Job {
            seat: from,
            kind: SnapshotKind::Rejoin,
            position: 0,
            created: now,
            phase: Phase::Await,
        });
        self.net.note(ResyncEvent::RejoinAccepted { slot: from });
        if let Some(r) = self.router.referee() {
            self.send_to(from, &Message::RefereeSeated { slot: Some(r) });
        }
        let last = self.router.last_record(from);
        self.send_to(
            from,
            &Message::Rejoined(crate::proto::Rejoined {
                last_turn: last,
                rejoin_token: new_token,
            }),
        );
    }

    fn on_resync_request(&mut self, from: PlayerId, r: ResyncRequest, now: Instant) {
        let host = self.router.host_seat();
        if !self.router.is_host() && !self.router.is_spectator() {
            return;
        }
        // The host never asks itself; the referee is asked by everyone.
        if from == host && !self.router.is_spectator() {
            return;
        }
        if !self.attempt_allowed(from, now) {
            return;
        }
        // The host with a referee in the room is not the authority (N5).
        if !self.serves() {
            return self.refuse(from, SnapshotRefusal::Unsupported);
        }
        if from == host && !crate::referee::resync_host(&self.net.referee_policy) {
            self.net.note(ResyncEvent::HostEndsMatch { slot: from });
            return self.refuse(from, SnapshotRefusal::HostEndsMatch);
        }
        let limits = self.net.host.limits.clone();
        let resyncs = self.net.host.ledger.get(usize::from(from.0)).map_or(0, |l| l.resyncs);
        if resyncs >= limits.resyncs_per_seat {
            self.net.note(ResyncEvent::LimitReached {
                slot: from,
                kind: SnapshotKind::Resync,
            });
            return self.refuse(from, SnapshotRefusal::LimitReached);
        }
        if self.net.host.jobs.iter().any(|j| j.seat == from) {
            return;
        }
        // The request must match a divergence the host knows of: this
        // seat's digest at that tick is the one it says and not the host's.
        let known = self.router.digests_at(r.tick);
        let mine = known.iter().find(|(p, _)| *p == self.router.me()).map(|x| x.1);
        let theirs = known.iter().find(|(p, _)| *p == from).map(|x| x.1);
        let plausible = r.position <= self.router.next_turn().saturating_add(256)
            && mine.is_some()
            // A seat that stopped at the mismatch sent no record with its
            // digest: what it says it holds counts when it is not the
            // authority's own.
            && theirs.map_or(self.router.is_spectator(), |t| t == r.digest)
            && mine != Some(r.digest);
        if !plausible {
            return self.refuse(from, SnapshotRefusal::NotDiverged);
        }
        if self.net.host.jobs.len() >= limits.max_jobs {
            return self.refuse(from, SnapshotRefusal::Busy);
        }
        if self.net.host.group.is_none() {
            let seats = self.router.seats();
            let expected = known
                .iter()
                .filter(|(p, d)| seats.contains(p) && *p != self.router.me() && Some(*d) != mine)
                .map(|(p, _)| *p)
                .collect();
            self.net.host.group = Some(Group {
                expected,
                until: now + limits.group_window,
                failed: false,
            });
        }
        // Counted when accepted, not when acked: a peer that asks and never
        // acks still uses up its resyncs.
        if let Some(l) = self.net.host.ledger(from) {
            l.resyncs += 1;
        }
        self.net.host.jobs.push(Job {
            seat: from,
            kind: SnapshotKind::Resync,
            position: r.position,
            created: now,
            phase: Phase::Await,
        });
    }

    fn on_ack(&mut self, from: PlayerId, a: ResyncAck) {
        let Some(i) = self.net.host.jobs.iter().position(|j| j.seat == from) else {
            return;
        };
        let Phase::Sent { digest, tick, turn, .. } = self.net.host.jobs[i].phase else {
            return;
        };
        let kind = self.net.host.jobs[i].kind;
        self.net.host.jobs.remove(i);
        if a.tick != tick || a.digest != digest {
            self.net.note(ResyncEvent::AckMismatch {
                slot: from,
                theirs: a.digest,
                ours: digest,
            });
            if kind == SnapshotKind::Resync
                && let Some(g) = self.net.host.group.as_mut()
            {
                g.failed = true;
            }
            // N4.4: the match ends as NET.md section 7 describes; the host
            // stays stopped, and for a rejoin the lobby kicks the seat.
            return;
        }
        match kind {
            SnapshotKind::Rejoin => {
                self.net.note(ResyncEvent::Arrived { slot: from, turn });
                self.router.send_message(Destination::All, &Message::Arrived(Arrived { slot: from, turn })).ok();
            }
            SnapshotKind::Resync => {
                let count = self.net.host.ledger(from).map_or(0, |l| l.resyncs);
                let last = {
                    let g = self.net.host.group.as_mut();
                    if let Some(g) = g {
                        g.expected.remove(&from);
                        g.expected.is_empty() && !g.failed
                    } else {
                        true
                    }
                } && self.net.host.jobs.iter().all(|j| j.kind != SnapshotKind::Resync);
                self.net.note(ResyncEvent::Resynced {
                    slot: from,
                    turn,
                    count,
                    last,
                });
                let msg = Message::Resynced(Resynced {
                    slot: from,
                    turn,
                    count,
                    last,
                });
                self.router.send_message(Destination::All, &msg).ok();
                if last {
                    self.net.host.group = None;
                    self.router.clear_desync_at(turn);
                }
            }
        }
    }

    fn host_timers(&mut self, now: Instant) {
        if self.net.host.jobs.is_empty() {
            self.net.host.cache = None;
        }
        let limits = self.net.host.limits.clone();
        let mut dead: Vec<(PlayerId, &'static str)> = Vec::new();
        for j in &self.net.host.jobs {
            let since = now.saturating_duration_since(j.created);
            match &j.phase {
                Phase::Await if since > limits.boundary_wait => dead.push((j.seat, "no usable turn boundary in time")),
                Phase::Send(t) if now.saturating_duration_since(t.began) > limits.send_deadline => {
                    dead.push((j.seat, "send deadline"));
                }
                Phase::Sent { at, .. } if now.saturating_duration_since(*at) > limits.ack_wait => {
                    dead.push((j.seat, "no ack"));
                }
                _ => {}
            }
        }
        for (seat, why) in dead {
            let was_await = self
                .net
                .host
                .jobs
                .iter()
                .find(|j| j.seat == seat)
                .is_some_and(|j| matches!(j.phase, Phase::Await));
            let kind = self.net.host.jobs.iter().find(|j| j.seat == seat).map(|j| j.kind);
            self.net.host.jobs.retain(|j| j.seat != seat);
            self.net.note(ResyncEvent::TransferFailed { slot: seat, why });
            if was_await {
                self.refuse(seat, SnapshotRefusal::Timeout);
            }
            if kind == Some(SnapshotKind::Rejoin) {
                // Still vacated: the seat may try again.
                self.net.host.vacated.insert(seat);
            }
        }
        // The window for the other diverged seats to ask has passed.
        let release = self.net.host.group.as_ref().is_some_and(|g| now >= g.until && !g.failed)
            && self.net.host.jobs.iter().all(|j| j.kind != SnapshotKind::Resync);
        if release {
            let slot = self
                .net
                .host
                .group
                .as_ref()
                .and_then(|g| g.expected.iter().next().copied())
                .unwrap_or(self.router.host_seat());
            self.net.host.group = None;
            let msg = Message::Resynced(Resynced {
                slot,
                turn: self.router.next_turn(),
                count: 0,
                last: true,
            });
            self.net.note(ResyncEvent::Resynced {
                slot,
                turn: self.router.next_turn(),
                count: 0,
                last: true,
            });
            self.router.send_message(Destination::All, &msg).ok();
            self.router.clear_desync();
        }
    }

    /// Sends what each transfer's budget allows.
    fn host_serve(&mut self, now: Instant) {
        let limits = self.net.host.limits.clone();
        let mut failed: Vec<PlayerId> = Vec::new();
        let mut finished: Vec<(PlayerId, usize)> = Vec::new();
        let n = self.net.host.jobs.len();
        // One budget for all transfers: the host's connection is limited.
        let sending = self.net.host.jobs.iter().filter(|j| matches!(j.phase, Phase::Send(_))).count().max(1);
        let rate = f64::from(limits.bytes_per_sec) / sending as f64;
        for i in 0..n {
            let seat = self.net.host.jobs[i].seat;
            let Phase::Send(t) = &mut self.net.host.jobs[i].phase else {
                continue;
            };
            let dt = now.saturating_duration_since(t.refilled).as_secs_f64();
            t.refilled = now;
            t.tokens = (t.tokens + dt * rate).min(f64::from(limits.burst_bytes) / sending as f64);
            let mut sent = 0;
            while t.cursor < t.len() && sent < 64 {
                let (msg, cost) = t.message(t.cursor);
                if t.tokens < cost as f64 && t.tokens < f64::from(limits.burst_bytes) / sending as f64 {
                    break;
                }
                if self
                    .router
                    .send_message(Destination::Seat(seat), &msg)
                    .is_err()
                {
                    failed.push(seat);
                    break;
                }
                t.tokens -= cost as f64;
                t.cursor += 1;
                sent += 1;
            }
            if t.cursor >= t.len() && !failed.contains(&seat) {
                finished.push((seat, t.bytes.len()));
            }
        }
        for seat in failed {
            if let Some(j) = self.net.host.jobs.iter().find(|j| j.seat == seat)
                && j.kind == SnapshotKind::Rejoin
            {
                self.net.host.vacated.insert(seat);
            }
            self.net.host.jobs.retain(|j| j.seat != seat);
            self.net.note(ResyncEvent::TransferFailed {
                slot: seat,
                why: "connection lost",
            });
        }
        for (seat, bytes) in finished {
            if let Some(j) = self.net.host.jobs.iter_mut().find(|j| j.seat == seat)
                && let Phase::Send(t) = &j.phase
            {
                let (digest, tick, turn) = (t.offer.digest, t.offer.tick, t.offer.seed.turn);
                let kind = j.kind;
                j.phase = Phase::Sent { at: now, digest, tick, turn };
                self.net.note(ResyncEvent::Sent { slot: seat, kind, bytes });
            }
        }
    }

    /// At a turn boundary, before the turn runs: start the transfers that
    /// are waiting for one. Called with the digests of the boundary known.
    pub(crate) fn net_boundary(&mut self, now: Instant) {
        if !(self.router.is_host() || self.router.is_spectator())
            || self.net.host.jobs.iter().all(|j| !matches!(j.phase, Phase::Await))
        {
            return;
        }
        let tpt = self.ticks_per_turn.max(1);
        if !self.tick.is_multiple_of(tpt) || self.router.next_turn() != u32::try_from(self.tick / tpt).unwrap_or(u32::MAX) {
            return;
        }
        let Some((seed, records)) = self.router.seed() else {
            return self.refuse_all_waiting(SnapshotRefusal::TooLarge);
        };
        let digest = self.sim.digest();
        let sections = self.sim.section_digests().values().to_vec();
        let mut bytes: Option<Arc<Vec<u8>>> = self
            .net
            .host
            .cache
            .as_ref()
            .filter(|(t, _)| *t == self.tick)
            .map(|(_, b)| Arc::clone(b));
        let limits = self.net.host.limits.clone();
        let seats = self.router.seats();
        let n = self.net.host.jobs.len();
        let mut refused: Vec<PlayerId> = Vec::new();
        for i in 0..n {
            if !matches!(self.net.host.jobs[i].phase, Phase::Await) {
                continue;
            }
            let (seat, kind, position) = {
                let j = &self.net.host.jobs[i];
                (j.seat, j.kind, j.position)
            };
            if kind == SnapshotKind::Resync
                && !seats
                    .iter()
                    .all(|&s| !self.router.sends(s) || self.router.seat_next(s).is_some_and(|n| n >= position))
            {
                // The requester consumed records the host has not read yet:
                // wait for them (they are on their way).
                continue;
            }
            let b = bytes.get_or_insert_with(|| {
                let b = Arc::new(self.sim.snapshot());
                self.net.host.cache = Some((self.tick, Arc::clone(&b)));
                b
            });
            if b.len() > crate::proto::MAX_SNAPSHOT_BYTES {
                refused.push(seat);
                continue;
            }
            let offer = SnapshotOffer {
                kind,
                tick: self.tick,
                format: FORMAT_VERSION,
                content_hash: self.content.hash(),
                digest,
                sections: sections.clone(),
                len: u32::try_from(b.len()).unwrap_or(u32::MAX),
                chunk: MAX_SNAPSHOT_PART_BYTES as u32,
                parts: u16::try_from(b.len().div_ceil(MAX_SNAPSHOT_PART_BYTES)).unwrap_or(u16::MAX),
                hash: snapshot_hash(b),
                seed: seed.clone(),
            };
            let transfer = Transfer {
                start: (kind == SnapshotKind::Rejoin).then(|| self.start.clone()),
                offer,
                bytes: Arc::clone(b),
                records: records.clone(),
                cursor: 0,
                tokens: f64::from(limits.burst_bytes),
                refilled: now,
                began: now,
            };
            self.net.host.jobs[i].phase = Phase::Send(Box::new(transfer));
        }
        for seat in refused {
            self.net.host.jobs.retain(|j| j.seat != seat);
            self.net.note(ResyncEvent::TransferFailed {
                slot: seat,
                why: "snapshot over the size bound",
            });
            self.refuse(seat, SnapshotRefusal::TooLarge);
        }
    }

    fn refuse_all_waiting(&mut self, why: SnapshotRefusal) {
        let seats: Vec<PlayerId> = self
            .net
            .host
            .jobs
            .iter()
            .filter(|j| matches!(j.phase, Phase::Await))
            .map(|j| j.seat)
            .collect();
        for s in seats {
            self.refuse(s, why);
            self.net.host.jobs.retain(|j| j.seat != s);
        }
    }
}
