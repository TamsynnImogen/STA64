//! Mission movie events to playback (MED-03).
//!
//! A mission raises `Presentation::Movie { name, in_panel, skippable }` for
//! `Media:playMovie` (MMM, full screen), `PlayCinematicMovie` (Armada II, the HUD's
//! cinematic panel, `in_panel`) and `PlayBridgeMovie` (Armada I: the bridge scene's screen,
//! [`BridgeScreen`], `in_panel`), and
//! `Presentation::StopMovie` for `StopMovie`. They arrive as `SimMsg` events;
//! this system turns them into [`PlayMovie`] / [`StopMovie`] messages. The
//! report that a movie is over is the [`MovieFinished`](super::MovieFinished)
//! message with the id returned in [`Requested`]; the client side that answers
//! the script with it is UI-30's, which reads [`Requested`] and `MovieFinished`.

use bevy::prelude::*;
use sta_sim::SimEvent;
use sta_sim::present::Presentation;

use super::{BridgeScreen, MovieId, MovieIds, MovieTarget, PanelScreen, PlayMovie, StopMovie};
use crate::sim_link::SimMsg;

/// A movie a mission asked for: its request id and the name the script used.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    pub id: MovieId,
    pub name: String,
}

/// Where a mission's movie goes. In Armada I (a [`BridgeScreen`] exists) `PlayBridgeMovie`, the only
/// call that raises an in-panel movie there, plays in the bridge scene's screen (read from the
/// executable, static: the call's mode selects the bridge display's rectangle, not the HUD's).
/// Elsewhere an in-panel movie goes to the HUD's panel, and a full-screen one (or one with no panel
/// to show it on) plays full screen.
pub(crate) fn target_for(in_panel: bool, bridge: Option<&BridgeScreen>, panel: Option<&PanelScreen>) -> MovieTarget {
    match (in_panel, bridge, panel) {
        (true, Some(b), _) => MovieTarget::Panel(b.image.clone()),
        (true, None, Some(p)) => MovieTarget::Panel(p.image.clone()),
        // No panel to show it on yet (the HUD has no cinematic panel): the whole screen stands in for it.
        _ => MovieTarget::Fullscreen,
    }
}

pub(super) fn bridge(
    mut events: MessageReader<SimMsg>,
    mut ids: ResMut<MovieIds>,
    panel: Option<Res<PanelScreen>>,
    bridge: Option<Res<BridgeScreen>>,
    mut play: MessageWriter<PlayMovie>,
    mut stop: MessageWriter<StopMovie>,
    mut requested: MessageWriter<Requested>,
) {
    for SimMsg(e) in events.read() {
        let SimEvent::Present(p) = e else { continue };
        match p {
            Presentation::Movie { name, in_panel, skippable } => {
                let target = target_for(*in_panel, bridge.as_deref(), panel.as_deref());
                let id = ids.next();
                requested.write(Requested { id, name: name.clone() });
                play.write(PlayMovie { id, name: name.clone(), target, skippable: *skippable });
            }
            Presentation::StopMovie => {
                stop.write(StopMovie(None));
            }
            _ => {}
        }
    }
}
