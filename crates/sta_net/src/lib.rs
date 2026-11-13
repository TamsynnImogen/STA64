//! Multiplayer: the lockstep wire protocol, a lockstep
//! `sta_sim::command::CommandRouter`, and the transport and relay.
//!
//! Owner: the NET lane (docs/WORKPLAN.md): NET-01 (protocol), NET-02
//! (lockstep router, `LoopbackNet`), NET-03 (transport and relay). The
//! design is docs/NET.md.

pub mod diagnose;
pub mod lockstep;
pub mod proto;
pub mod resync;
pub mod transport;
