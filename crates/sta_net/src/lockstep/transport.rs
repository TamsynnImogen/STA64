//! The seam between the router and a network, and the in-process transport
//! that stands in for one.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use sta_core::PlayerId;

use crate::proto::{self, Message, ProtoError};

/// Where a message goes. The relay delivers to the room's other members only,
/// never back to the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// Every other seat of the room.
    All,
    /// One seat (a non-host's `Request` goes to the host).
    Seat(PlayerId),
}

/// A message and who the transport says sent it (NET-03: the relay's stamp).
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    pub from: PlayerId,
    pub msg: Message,
}

/// Why a transport could not send or deliver.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportError {
    /// The connection is gone.
    Closed,
    /// A frame did not encode or decode. A real transport disconnects the
    /// sender.
    Frame(ProtoError),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Closed => write!(f, "connection closed"),
            TransportError::Frame(e) => write!(f, "bad frame: {e}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// What the router needs from a network: reliable delivery, in order for each
/// sender (docs/NET.md, section 1: the relay forwards each sender's frames in
/// the order it received them). Different senders' frames may interleave in
/// any order, and may arrive late.
///
/// NET-03 implements it over the relay; [`LoopbackLink`] over a channel.
pub trait Transport: Send {
    /// Queues `msg` for `to`. Never blocks.
    fn send(&mut self, to: Destination, msg: &Message) -> Result<(), TransportError>;
    /// The next message that has arrived, if any. Never blocks.
    fn poll(&mut self) -> Result<Option<Incoming>, TransportError>;
}

/// A fault injected into the loopback, for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFault {
    /// Frames from this seat are held back (a late or silent peer) until
    /// [`LoopbackNet::release`]; they are delivered then, in order.
    Hold(PlayerId),
    /// Swap the delivery order of the next two frames this seat sends to
    /// each receiver: a transport bug the router must refuse.
    SwapNextPair(PlayerId),
}

#[derive(Debug)]
struct Frame {
    deliver_at: u64,
    from: PlayerId,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct Hub {
    /// Logical clock; frames are delivered when it reaches their time.
    now: u64,
    /// Per receiver, in delivery order.
    inbox: BTreeMap<PlayerId, VecDeque<Frame>>,
    /// Latest delivery time per (sender, receiver) link: keeps each link
    /// FIFO whatever the jitter.
    link_clock: BTreeMap<(PlayerId, PlayerId), u64>,
    latency: u64,
    jitter: u64,
    rng: u64,
    held: BTreeSet<PlayerId>,
    swap: BTreeSet<PlayerId>,
    /// Frames held back by `Hold`, per sender, oldest first.
    held_frames: BTreeMap<PlayerId, Vec<(PlayerId, Vec<u8>)>>,
    /// Seats whose connection ended (NET-04): nothing is delivered to them
    /// until [`LoopbackNet::reconnect`].
    vacated: BTreeSet<PlayerId>,
    sent: u64,
}

impl Hub {
    fn next_rand(&mut self) -> u64 {
        // SplitMix64.
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn deliver(&mut self, from: PlayerId, to: PlayerId, bytes: Vec<u8>) {
        let jitter = if self.jitter == 0 {
            0
        } else {
            self.next_rand() % (self.jitter + 1)
        };
        let earliest = self.now + self.latency + jitter;
        let clock = self.link_clock.entry((from, to)).or_insert(0);
        let deliver_at = earliest.max(*clock);
        *clock = deliver_at;
        let q = self.inbox.entry(to).or_default();
        let mut frame = Frame { deliver_at, from, bytes };
        let prev = q.iter().rposition(|f| f.from == from);
        match prev {
            Some(at) if self.swap.contains(&from) => {
                // Put this frame in front of the sender's previous one, due
                // at the same time.
                self.swap.remove(&from);
                frame.deliver_at = q[at].deliver_at;
                q.insert(at, frame);
            }
            _ => q.push_back(frame),
        }
    }
}

/// An in-process room: one [`LoopbackLink`] per seat over shared queues.
/// Every message goes through [`proto::encode`] and [`proto::decode`].
///
/// Delivery is driven by a logical clock ([`LoopbackNet::advance`]): a frame
/// sent at time `t` arrives at `t + latency + jitter`, never before an
/// earlier frame of the same sender on the same link, but in any order
/// relative to other senders' frames. With latency 0 and jitter 0 a frame is
/// there as soon as it is sent.
#[derive(Debug, Clone)]
pub struct LoopbackNet {
    hub: Arc<Mutex<Hub>>,
    seats: Arc<[PlayerId]>,
}

impl LoopbackNet {
    /// A room with `seats`, zero latency, and no faults.
    pub fn new(seats: &[PlayerId]) -> (LoopbackNet, Vec<LoopbackLink>) {
        LoopbackNet::with_delay(seats, 0, 0, 0)
    }

    /// A room whose frames take `latency` plus a pseudo-random `0..=jitter`
    /// clock units (from `seed`) to arrive.
    pub fn with_delay(
        seats: &[PlayerId],
        latency: u64,
        jitter: u64,
        seed: u64,
    ) -> (LoopbackNet, Vec<LoopbackLink>) {
        let hub = Hub {
            latency,
            jitter,
            rng: seed,
            ..Hub::default()
        };
        let net = LoopbackNet {
            hub: Arc::new(Mutex::new(hub)),
            seats: seats.into(),
        };
        let links = seats
            .iter()
            .map(|&me| LoopbackLink {
                me,
                seats: seats.to_vec(),
                hub: net.hub.clone(),
            })
            .collect();
        (net, links)
    }

    fn hub(&self) -> std::sync::MutexGuard<'_, Hub> {
        self.hub.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves the clock forward.
    pub fn advance(&self, by: u64) {
        self.hub().now += by;
    }

    /// The clock.
    pub fn now(&self) -> u64 {
        self.hub().now
    }

    /// Frames sent so far, one per recipient.
    pub fn frames_sent(&self) -> u64 {
        self.hub().sent
    }

    /// Injects a fault.
    pub fn inject(&self, fault: LinkFault) {
        let mut hub = self.hub();
        match fault {
            LinkFault::Hold(p) => {
                hub.held.insert(p);
            }
            LinkFault::SwapNextPair(p) => {
                hub.swap.insert(p);
            }
        }
    }

    /// Ends `seat`'s connection as a lost one: what was queued for it is
    /// gone, nothing more is delivered to it, and what it sent earlier stays
    /// delivered to the others, in order. The layer that reads the relay's
    /// departure marker tells its peer (`Peer::note_departed`).
    pub fn vacate(&self, seat: PlayerId) {
        let mut hub = self.hub();
        hub.vacated.insert(seat);
        hub.inbox.remove(&seat);
        hub.held.remove(&seat);
        hub.held_frames.remove(&seat);
    }

    /// A new connection for a vacated `seat` (the relay's rejoin): nothing
    /// that was sent while it was gone is delivered to it.
    pub fn reconnect(&self, seat: PlayerId) -> Option<LoopbackLink> {
        if !self.hub().vacated.remove(&seat) || !self.seats.contains(&seat) {
            return None;
        }
        Some(LoopbackLink {
            me: seat,
            seats: self.seats.to_vec(),
            hub: self.hub.clone(),
        })
    }

    /// Ends a [`LinkFault::Hold`]: what the seat sent meanwhile is delivered
    /// now, in order.
    pub fn release(&self, seat: PlayerId) {
        let mut hub = self.hub();
        hub.held.remove(&seat);
        for (to, bytes) in hub.held_frames.remove(&seat).unwrap_or_default() {
            hub.deliver(seat, to, bytes);
        }
    }
}

/// One seat's end of a [`LoopbackNet`].
#[derive(Debug)]
pub struct LoopbackLink {
    me: PlayerId,
    seats: Vec<PlayerId>,
    hub: Arc<Mutex<Hub>>,
}

impl LoopbackLink {
    /// The seat this link belongs to.
    pub fn seat(&self) -> PlayerId {
        self.me
    }
}

impl Transport for LoopbackLink {
    fn send(&mut self, to: Destination, msg: &Message) -> Result<(), TransportError> {
        let bytes = proto::encode(msg).map_err(TransportError::Frame)?;
        let mut hub = self.hub.lock().unwrap_or_else(PoisonError::into_inner);
        if hub.vacated.contains(&self.me) {
            return Err(TransportError::Closed);
        }
        let targets: Vec<PlayerId> = match to {
            Destination::All => self.seats.iter().copied().filter(|&s| s != self.me).collect(),
            Destination::Seat(s) if self.seats.contains(&s) && s != self.me => vec![s],
            Destination::Seat(_) => return Err(TransportError::Closed),
        };
        for t in targets {
            if hub.vacated.contains(&t) {
                continue;
            }
            hub.sent += 1;
            if hub.held.contains(&self.me) {
                hub.held_frames
                    .entry(self.me)
                    .or_default()
                    .push((t, bytes.clone()));
            } else {
                hub.deliver(self.me, t, bytes.clone());
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<Option<Incoming>, TransportError> {
        let mut hub = self.hub.lock().unwrap_or_else(PoisonError::into_inner);
        if hub.vacated.contains(&self.me) {
            return Err(TransportError::Closed);
        }
        let now = hub.now;
        let Some(q) = hub.inbox.get_mut(&self.me) else {
            return Ok(None);
        };
        // Deliver the first frame that is due; later frames of the same
        // sender never overtake it because their times are not earlier.
        let Some(i) = q.iter().position(|f| f.deliver_at <= now) else {
            return Ok(None);
        };
        let f = q.remove(i).expect("position is in range");
        let msg = proto::decode(&f.bytes).map_err(TransportError::Frame)?;
        Ok(Some(Incoming { from: f.from, msg }))
    }
}
