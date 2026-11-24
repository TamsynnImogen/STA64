//! Anything that turns commands into world views (S3): the real [`Sim`],
//! and the fakes [`ScriptedSource`] and [`ViewLog`]. The client's sim link,
//! the bench and net step a `dyn WorldSource` and never care which.
//!
//! [`Sim`]: crate::Sim
//! [`ScriptedSource`]: crate::fake::ScriptedSource
//! [`ViewLog`]: crate::fake::ViewLog

use std::sync::Arc;

use crate::command::Command;
use crate::view::{TickOutput, WorldView};

/// Optional authored values for a paused map object. Missing values retain the
/// definition's current simulation defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ObjectProperties {
    pub hull: Option<[crate::Fixed; 2]>,
    pub shields: Option<[[crate::Fixed; 2]; 4]>,
    pub crew: Option<[crate::Fixed; 2]>,
    pub energy: Option<crate::Fixed>,
    /// Fraction of health remaining, in `SystemView` order.
    pub systems: Option<[crate::Fixed; 5]>,
    pub resource: Option<crate::Fixed>,
    pub population: Option<crate::Fixed>,
    pub captain: Option<String>,
    pub registry: Option<String>,
    /// The original instance display name, when separate from the map label.
    pub instance_name: Option<String>,
}

/// Placement tools for a local authoring or test-play session. These take
/// effect without advancing the game clock.
pub enum PlacementEdit {
    Place {
        def: crate::ContentKey,
        owner: crate::PlayerId,
        at: crate::Vec3,
    },
    Transform {
        id: crate::EntityId,
        at: crate::Vec3,
        quarter_turns: i32,
    },
    Inspect {
        id: crate::EntityId,
        owner: crate::PlayerId,
        at: crate::Vec3,
        heading: (crate::Fixed, crate::Fixed),
        label: Option<String>,
        name: Option<String>,
        properties: ObjectProperties,
    },
    Remove(crate::EntityId),
}

/// Definitions the editor can place as simulated objects. Map regions and
/// background scenery have their own representation in the map loader.
pub fn editable_placement(def: &sta_content::UnitDef) -> bool {
    use crate::world::EntityKind;
    !EntityKind::hidden(def)
        && def.a1_officer_upgrade_gain.is_none()
        && EntityKind::of_def(def) != EntityKind::Projectile
        && !matches!(def.class.0.as_str(), "asteroidfield" | "nebula_obj" | "background_obj")
}

/// Steps a world, one tick per call.
pub trait WorldSource: Send {
    fn supports_placement_edits(&self) -> bool {
        false
    }

    /// The affected entity, or None if this source or placement is unsupported.
    fn edit_placement(&mut self, _edit: PlacementEdit) -> Option<crate::EntityId> {
        None
    }

    /// Advance one tick, applying `commands` (sorted by `(player, seq)`).
    fn step(&mut self, commands: &[Command]) -> TickOutput;
    /// The world now: after the last step, or at tick 0 before any.
    fn view(&self) -> Arc<WorldView>;
    /// Optional immutable queue/site metadata for presentation. Sources without
    /// production state (including legacy view logs) return no rows. Command
    /// replays and snapshot restores derive it from their live `Sim`.
    fn production_view(&self) -> Vec<crate::production::view::ProductionView> {
        Vec::new()
    }
    /// Optional immutable cargo metadata for presentation. Stateless sources
    /// return no rows; live simulations derive it from their economy records.
    fn cargo_view(&self) -> Vec<crate::economy::view::CargoView> {
        Vec::new()
    }
    /// Optional resource-node amounts for presentation (UI-48). Stateless sources return no rows.
    fn node_view(&self) -> Vec<crate::economy::view::NodeView> {
        Vec::new()
    }
    /// Optional system states for presentation (UI-48). Stateless sources return no rows.
    fn system_view(&self) -> Vec<crate::combat::damage::SystemView> {
        Vec::new()
    }
    /// The whole world as snapshot bytes ([`Sim::snapshot`]), for sources that
    /// can: the playtest capture (TOOL-14) reads it, nothing in the sim does.
    /// The fakes have no state to save and answer `None`.
    ///
    /// [`Sim::snapshot`]: crate::Sim::snapshot
    fn snapshot(&self) -> Option<Vec<u8>> {
        None
    }
}
