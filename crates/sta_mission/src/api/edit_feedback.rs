//! Rejections of selected script edits, correlated only after host submission.

use std::collections::BTreeMap;

use sta_core::{PlayerId, Tick};
use sta_sim::SimEvent;
use sta_sim::command::{Command, CommandKind, Edit};

use super::Ctx;

struct Tracked {
    label: &'static str,
    edit: Edit,
    submitted: Option<Tick>,
}

#[derive(Default)]
pub(super) struct Store {
    tracked: BTreeMap<u32, Tracked>,
}

pub(super) fn queue(ctx: &mut Ctx, label: &'static str, edit: Edit) -> mlua::Result<()> {
    let seq = ctx
        .try_push(PlayerId::SCRIPT, CommandKind::Edit(edit.clone()))
        .map_err(mlua::Error::runtime)?;
    ctx.edit_feedback.tracked.insert(
        seq,
        Tracked {
            label,
            edit,
            submitted: None,
        },
    );
    Ok(())
}

impl Store {
    pub(super) fn submit(&mut self, commands: &[Command], tick: Tick) {
        for command in commands {
            if command.player == PlayerId::SCRIPT
                && let Some(tracked) = self.tracked.get_mut(&command.seq)
                && matches!(&command.kind, CommandKind::Edit(edit) if *edit == tracked.edit)
            {
                tracked.submitted = Some(tick);
            }
        }
    }

    pub(super) fn process(&mut self, tick: Tick, events: &[SimEvent]) -> Vec<(&'static str, String)> {
        let settled: Vec<_> = self
            .tracked
            .iter()
            .filter_map(|(&seq, tracked)| tracked.submitted.filter(|&at| tick > at).map(|_| seq))
            .collect();
        let mut failures = Vec::new();
        for seq in settled {
            let tracked = self.tracked.remove(&seq).expect("listed tracked edit");
            if events.iter().any(|event| {
                matches!(event,
                SimEvent::Rejected { player: PlayerId::SCRIPT, seq: rejected } if *rejected == seq)
            }) {
                failures.push((
                    tracked.label,
                    format!(
                        "{}: simulation rejected SCRIPT seq {} {:?}",
                        tracked.label, seq, tracked.edit
                    ),
                ));
            }
        }
        failures
    }
}

pub(super) fn process(ctx: &mut Ctx) -> Vec<String> {
    let failures = ctx.edit_feedback.process(ctx.view.tick, &ctx.events);
    for &(label, _) in &failures {
        ctx.unsupported(&format!("{label}: simulation rejected edit"));
    }
    failures.into_iter().map(|(_, message)| message).collect()
}
