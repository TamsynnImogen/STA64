//! The network (NET-03): a relay with rooms, and the client transport the
//! lockstep router uses over it.
//!
//! - [`wire`]: the envelope between client and relay and the bounded stream
//!   reader. The relay reads [`proto`](crate::proto) frame headers only.
//! - [`Relay`]: rooms with join codes, a star of TCP connections, stamping
//!   of the sender, directed delivery, departure markers, limits.
//! - [`NetTransport`]: the real [`Transport`](crate::lockstep::Transport);
//!   [`NetControl`] carries what a lobby needs beyond it.
//!
//! Transport choice (docs/NET.md leaves it open: "reliable ordered frames"):
//! TCP, standard library only. Lockstep needs reliable, ordered delivery per
//! sender, which TCP gives, and needs no extra dependency. Loss and reorder
//! therefore do not happen at this layer; slowness does, and is what the
//! router's stall handling is for. A QUIC or WebSocket transport would be one
//! more implementation of the same trait.

pub mod client;
pub mod relay;
pub mod wire;

pub use client::{
    CloseReason, ClientOptions, ConnectError, NetControl, NetTransport,
};
pub use relay::{Relay, RelayConfig};
pub use wire::{RefuseReason, RoomCode};
