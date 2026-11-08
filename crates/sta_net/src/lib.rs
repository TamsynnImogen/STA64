//! Multiplayer: the lockstep wire protocol, a lockstep
//! `sta_sim::command::CommandRouter`, and the transport and relay.
//!
//! Owner: the NET lane (docs/WORKPLAN.md): NET-01 (protocol), NET-02
//! (lockstep router, `LoopbackNet`), NET-03 (transport and relay). Created
//! empty by R0-2 so the workspace members are fixed; the lane fills it.
