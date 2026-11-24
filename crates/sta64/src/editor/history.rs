//! Undo and redo restore the paused simulation and its map document together.

use super::{AuthoredIds, Document};
use crate::sim_link::{ContentRes, Selection, SimLink};
use crate::ui::hud::instance_names::InstanceNames;
use sta_sim::EntityId;

const LIMIT: usize = 16;

pub(super) struct Checkpoint {
    simulation: Vec<u8>,
    document: Option<Document>,
    ids: Option<AuthoredIds>,
    names: Option<InstanceNames>,
    selection: Vec<EntityId>,
}

impl Checkpoint {
    pub(super) fn take(
        link: &mut SimLink,
        document: Option<&Document>,
        ids: Option<&AuthoredIds>,
        names: Option<&InstanceNames>,
        selection: &Selection,
    ) -> Option<Self> {
        Some(Self {
            simulation: link.snapshot()?,
            document: document.cloned(),
            ids: ids.cloned(),
            names: names.cloned(),
            selection: selection.0.clone(),
        })
    }
}

#[derive(Default)]
pub(super) struct History {
    undo: Vec<Checkpoint>,
    redo: Vec<Checkpoint>,
}

impl History {
    pub(super) fn record(&mut self, before: Option<Checkpoint>) {
        if let Some(before) = before {
            if self.undo.len() == LIMIT {
                self.undo.remove(0);
            }
            self.undo.push(before);
            self.redo.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn restore(
        &mut self,
        redo: bool,
        link: &mut SimLink,
        db: &ContentRes,
        document: Option<&mut Document>,
        ids: Option<&mut AuthoredIds>,
        names: Option<&mut InstanceNames>,
        selection: &mut Selection,
    ) -> Result<bool, String> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let Some(checkpoint) = from.last() else {
            return Ok(false);
        };
        let current = Checkpoint::take(link, document.as_deref(), ids.as_deref(), names.as_deref(), selection)
            .ok_or("This world cannot be checkpointed")?;
        link.restore_editor_snapshot(&checkpoint.simulation, db.0.clone()).map_err(|error| error.to_string())?;
        let checkpoint = from.pop().expect("checkpoint was read above");
        if let (Some(document), Some(saved)) = (document, checkpoint.document) {
            *document = saved;
        }
        if let (Some(ids), Some(saved)) = (ids, checkpoint.ids) {
            *ids = saved;
        }
        if let (Some(names), Some(saved)) = (names, checkpoint.names) {
            *names = saved;
        }
        selection.0 = checkpoint.selection;
        to.push(current);
        Ok(true)
    }
}
