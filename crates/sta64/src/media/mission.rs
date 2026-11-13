//! Mission movie events to playback (MED-03).
//!
//! A mission raises `Presentation::Movie { name, in_panel, skippable }` for
//! `Media:playMovie` (MMM), `PlayCinematicMovie` (full screen) and
//! `PlayBridgeMovie` (the bridge viewscreen, `in_panel`), and
//! `Presentation::StopMovie` for `StopMovie`. They arrive as `SimMsg` events;
//! this system turns them into [`PlayMovie`] / [`StopMovie`] messages. The
//! report that a movie is over is the [`MovieFinished`](super::MovieFinished)
//! message with the id returned in [`Requested`]; the client side that answers
//! the script with it is UI-30's, which reads [`Requested`] and `MovieFinished`.

use bevy::prelude::*;
use sta_sim::SimEvent;
use sta_sim::present::Presentation;

use super::{MovieId, MovieIds, MovieTarget, PanelScreen, PlayMovie, StopMovie};
use crate::sim_link::SimMsg;

/// A movie a mission asked for: its request id and the name the script used.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    pub id: MovieId,
    pub name: String,
}

pub(super) fn bridge(
    mut events: MessageReader<SimMsg>,
    mut ids: ResMut<MovieIds>,
    panel: Option<Res<PanelScreen>>,
    mut play: MessageWriter<PlayMovie>,
    mut stop: MessageWriter<StopMovie>,
    mut requested: MessageWriter<Requested>,
) {
    for SimMsg(e) in events.read() {
        let SimEvent::Present(p) = e else { continue };
        match p {
            Presentation::Movie { name, in_panel, skippable } => {
                let target = match (in_panel, &panel) {
                    (true, Some(p)) => MovieTarget::Panel(p.0.clone()),
                    // No viewscreen to show it on: the whole screen is the viewscreen.
                    _ => MovieTarget::Fullscreen,
                };
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
