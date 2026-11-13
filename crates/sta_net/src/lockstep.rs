//! The lockstep router (NET-02): the [`CommandRouter`](sta_sim::command::CommandRouter)
//! that holds a turn until every seated peer's record for it has arrived, the
//! transport seam NET-03 implements, an in-process transport
//! ([`LoopbackNet`]) and a driver that runs one sim on top of a router
//! ([`Peer`]). The design is docs/NET.md; sections 3, 6, 7 and 8 are what this
//! module builds.
//!
//! - [`LockstepRouter`] is the router of one peer. It sends one
//!   [`TurnCommands`](crate::proto::TurnCommands) per turn, assembles the
//!   others' records, checks them (role, authority, gap-free order,
//!   increasing `seq`), compares digests and halts on a mismatch.
//! - [`Transport`] is all the router knows about a network: send to all or to
//!   one seat, poll what arrived, with the sender stamped by the transport
//!   (the relay's stamp). It speaks [`Message`](crate::proto::Message)s; the
//!   transport owns framing. [`LoopbackNet`] encodes and decodes every
//!   message through the real frame, so bounds are exercised.
//! - [`Peer`] steps a [`Sim`](sta_sim::Sim) through a router and keeps the
//!   replay that is the desync report.
//! - [`admit`] is the host's side of the handshake (docs/NET.md, section 5),
//!   as a function of values, so two in-process peers can be refused with the
//!   differing key named before a game starts.
//!
//! Depends only on the public `sta_sim` seam (`Command`, `CommandRouter`,
//! `Sim::digest`, `Replay`), not on snapshot internals.

mod handshake;
mod peer;
mod router;
mod transport;

pub use handshake::{admit, HandshakeError};
pub use peer::{DesyncFiles, Fault, Peer, StepOutcome};
pub use router::{LockstepRouter, RouterError, RouterEvent, DIGESTS_KEPT};
pub use transport::{
    Destination, Incoming, LinkFault, LoopbackLink, LoopbackNet, Transport, TransportError,
};
