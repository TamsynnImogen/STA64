//! Mission-side references to existing entities and queued spawn requests.

use std::cell::Cell;
use std::rc::Rc;

use sta_core::{EntityId, Tick};
use sta_sim::view::UnitView;

#[derive(Debug, Clone)]
pub(crate) enum EntityRef {
    Existing(EntityId),
    Spawn(Rc<SpawnHandle>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpawnStatus {
    Queued,
    Submitted(Tick),
    Resolved(EntityId),
    Failed,
}

#[derive(Debug)]
pub(crate) struct SpawnHandle {
    pub request: u64,
    pub seq: u32,
    pub hint: UnitView,
    /// Updated only by the host's command/receipt lifecycle, never by Lua.
    pub state: Cell<SpawnStatus>,
}

impl EntityRef {
    /// Returns an actual entity ID once this reference has resolved.
    pub fn id(&self) -> Option<EntityId> {
        match self {
            Self::Existing(id) => Some(*id),
            Self::Spawn(handle) => match handle.state.get() {
                SpawnStatus::Resolved(id) => Some(id),
                SpawnStatus::Queued | SpawnStatus::Submitted(_) | SpawnStatus::Failed => None,
            },
        }
    }

    /// Whether a spawn request is still waiting for a host receipt.
    pub fn pending(&self) -> bool {
        matches!(
            self,
            Self::Spawn(handle)
                if matches!(handle.state.get(), SpawnStatus::Queued | SpawnStatus::Submitted(_))
        )
    }

    /// Returns uncommitted request data while a spawn is queued or submitted.
    /// The hint's `id` is not an actual entity ID and must never be exposed as one.
    pub fn hint(&self) -> Option<&UnitView> {
        match self {
            Self::Spawn(handle) if matches!(handle.state.get(), SpawnStatus::Queued | SpawnStatus::Submitted(_)) => {
                Some(&handle.hint)
            }
            Self::Existing(_) | Self::Spawn(_) => None,
        }
    }

    /// Compares actual IDs when available, otherwise the identity of the request handle.
    pub fn same_entity(&self, other: &Self) -> bool {
        match (self.id(), other.id()) {
            (Some(left), Some(right)) => left == right,
            (None, None) => match (self, other) {
                (Self::Spawn(left), Self::Spawn(right)) => Rc::ptr_eq(left, right),
                _ => false,
            },
            _ => false,
        }
    }
}
