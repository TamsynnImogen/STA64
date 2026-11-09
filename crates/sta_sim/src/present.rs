//! Presentation: what a mission shows and plays without changing the world
//! (seam addition for LUA-05/LUA-06; docs/MISSION_DLL_API.md).
//!
//! Stock missions spend much of their engine calls on cineractives, camera
//! shots, fades, titles, audio messages, music and movies. None of it changes
//! the simulation, but every client and every replay must show it, and a
//! mission's only output is [`Command`](crate::command::Command)s. So a
//! mission sends [`Edit::Present`](crate::command::Edit::Present) from
//! [`PlayerId::SCRIPT`](sta_core::PlayerId::SCRIPT); the sim changes nothing
//! and raises [`SimEvent::Present`](crate::event::SimEvent::Present) that
//! tick. It travels in the command log like any edit, so lockstep peers and
//! replays see the same presentation, and the client's render, audio and
//! media systems consume it (they log what they cannot show yet).
//!
//! Names are the content's own (sound, music, movie and sprite names, map
//! path and label names) and resolve on the client. Durations are seconds.

use serde::{Deserialize, Serialize};
use sta_core::{EntityId, Fixed, Vec3};

/// A place a camera looks from or at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Spot {
    Entity(EntityId),
    /// A map path's point `index` (0 = its start).
    PathPoint { path: String, index: u32 },
    /// A whole map path, travelled over the shot.
    Path(String),
    Position(Vec3),
}

/// One argument of a [`Presentation::Call`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Arg {
    Bool(bool),
    Int(i64),
    Num(Fixed),
    Text(String),
    Entity(EntityId),
    Position(Vec3),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Presentation {
    /// Cineractive mode on (letterbox, input locked, `skippable` by the
    /// player) or off.
    Cineractive { on: bool, skippable: bool },
    /// A camera shot: from `from` looking at `at`, over `seconds`.
    Shot { from: Spot, at: Spot, seconds: Fixed },
    /// Offset of the camera from what it follows.
    CameraOffset { offset: Vec3, relative: bool },
    /// Fade the screen to (`out`) or from colour `rgb` over `seconds`.
    Fade { out: bool, seconds: Fixed, rgb: [u8; 3] },
    /// Show a title or subtitle (text by string id or literal) for `seconds`
    /// (`None`: until removed).
    Title { id: u32, text: String, subtitle: bool, seconds: Option<Fixed> },
    RemoveTitle { id: u32 },
    /// A voice or message sound; `id` lets a script stop it.
    Sound { id: u32, name: String, at: Option<EntityId> },
    StopSound { id: u32 },
    Music { name: String, looped: bool },
    StopMusic,
    /// A movie, full screen or in a panel (the bridge viewscreen).
    Movie { name: String, in_panel: bool, skippable: bool },
    StopMovie,
    /// A visual effect on an entity (`"transporter"`, ...).
    Effect { kind: String, on: EntityId },
    /// Wait for the player to press Enter; the client answers with a
    /// presentation-done command (LUA-05) carrying `id`.
    WaitForKey { id: u32 },
    /// Anything not typed yet, by the engine call's name: kept so nothing a
    /// script asks for is lost, and typed when a consumer needs it.
    Call { name: String, args: Vec<Arg> },
}
