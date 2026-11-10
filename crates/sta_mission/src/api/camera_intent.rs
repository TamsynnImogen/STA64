//! Typed camera spot requests. Deferred requests contain no Lua values or
//! predicted IDs and are ordered within the current camera-source scene.

use std::collections::VecDeque;

use sta_core::{Fixed, PlayerId, Vec3};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation, Spot};

use super::Ctx;
use super::entity_ref::EntityRef;

#[derive(Clone)]
pub(crate) enum SpotRef {
    Entity(EntityRef),
    Path(String),
    Position(Vec3),
}

impl SpotRef {
    pub(crate) fn unresolved(&self) -> bool {
        matches!(self, Self::Entity(key) if key.id().is_none())
    }

    fn entity(&self) -> Option<&EntityRef> {
        match self {
            Self::Entity(key) => Some(key),
            _ => None,
        }
    }

    fn spot(&self) -> Option<Spot> {
        Some(match self {
            Self::Entity(key) => Spot::Entity(key.id()?),
            Self::Path(name) => Spot::Path(name.clone()),
            Self::Position(pos) => Spot::Position(*pos),
        })
    }

    fn arg(&self) -> Option<Arg> {
        Some(match self {
            Self::Entity(key) => Arg::Entity(key.id()?),
            Self::Path(name) => Arg::Text(name.clone()),
            Self::Position(pos) => Arg::Position(*pos),
        })
    }
}

pub(crate) enum ArgRef {
    Ready(Arg),
    Spot(SpotRef),
}

pub(crate) enum Intent {
    Call { name: &'static str, args: Vec<ArgRef> },
    Shot { from: SpotRef, at: SpotRef },
}

impl Intent {
    fn entities(&self) -> Vec<&EntityRef> {
        match self {
            Self::Shot { from, at } => [from.entity(), at.entity()].into_iter().flatten().collect(),
            Self::Call { args, .. } => args
                .iter()
                .filter_map(|arg| match arg {
                    ArgRef::Spot(spot) => spot.entity(),
                    ArgRef::Ready(_) => None,
                })
                .collect(),
        }
    }

    fn presentation(&self) -> Option<Presentation> {
        Some(match self {
            Self::Shot { from, at } => Presentation::Shot {
                from: from.spot()?,
                at: at.spot()?,
                seconds: Fixed::ZERO,
            },
            Self::Call { name, args } => Presentation::Call {
                name: (*name).into(),
                args: args
                    .iter()
                    .map(|arg| match arg {
                        ArgRef::Ready(arg) => Some(arg.clone()),
                        ArgRef::Spot(spot) => spot.arg(),
                    })
                    .collect::<Option<_>>()?,
            },
        })
    }
}

#[derive(Default)]
pub(crate) struct Store {
    queue: VecDeque<Intent>,
}

/// A new validated source or an explicit cinematic stop cancels older spots.
pub(crate) fn clear(c: &mut Ctx) {
    c.camera.queue.clear();
}

pub(crate) fn submit(c: &mut Ctx, intent: Intent) -> Result<(), &'static str> {
    let entities = intent.entities();
    if entities.iter().any(|e| e.id().is_none() && !e.pending()) {
        return Err("camera entity spawn failed");
    }
    if !c.camera.queue.is_empty() || entities.iter().any(|e| e.pending()) {
        c.camera.queue.push_back(intent);
    } else {
        let presentation = intent.presentation().ok_or("camera entity has no resolved ID")?;
        c.try_push(PlayerId::SCRIPT, CommandKind::Edit(Edit::Present(presentation)))?;
    }
    Ok(())
}

/// Called after spawn receipts resolve, before this tick's Lua callbacks.
pub(crate) fn process(c: &mut Ctx) {
    let mut queue = std::mem::take(&mut c.camera.queue);
    while let Some(intent) = queue.pop_front() {
        let entities = intent.entities();
        if entities.iter().any(|e| e.id().is_none() && !e.pending()) {
            c.unsupported("camera: deferred scene has a failed spawn");
            break;
        }
        if entities.iter().any(|e| e.pending()) {
            queue.push_front(intent);
            c.camera.queue = queue;
            break;
        }
        if entities.iter().any(|e| e.id().is_none_or(|id| c.alive(id).is_none())) {
            c.unsupported("camera: deferred scene entity no longer alive");
            break;
        }
        let Some(presentation) = intent.presentation() else {
            c.unsupported("camera: deferred scene has no resolved ID");
            break;
        };
        if let Err(reason) = c.try_push(PlayerId::SCRIPT, CommandKind::Edit(Edit::Present(presentation))) {
            c.unsupported(reason);
            break;
        }
    }
}
