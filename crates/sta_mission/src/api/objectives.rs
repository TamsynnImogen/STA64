//! File-backed host mission objectives. The caller's reader supplies bytes;
//! no filesystem access or game text is embedded in this module.

use super::{Objective, ObjectiveBriefing, ObjectiveState, Shared};

pub(super) fn load(ctx: &Shared, name: &str) -> mlua::Result<()> {
    load_with_completion(ctx, name, true)
}

/// Prepare the completion policy before committing a successful replacement.
/// Preserve mode carries only completed slots, never IDs or failure states.
/// All read/parse/ID failures retain the entire old list and metadata.
pub(super) fn load_with_completion(ctx: &Shared, name: &str, reset_completed: bool) -> mlua::Result<()> {
    let path = name.replace('\\', "/");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || path.split('/').any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(mlua::Error::runtime("Objectives.load requires a relative content path"));
    }
    let primary = if path.to_ascii_lowercase().starts_with("objectives/") {
        path.clone()
    } else {
        format!("objectives/{path}")
    };
    let reader = ctx.borrow().reader.clone();
    let found = reader.and_then(|reader| {
        let read = reader.lock().unwrap_or_else(|p| p.into_inner());
        read(&primary)
            .map(|b| (primary, b))
            .or_else(|| read(&path).map(|b| (path, b)))
    });
    let Some((file, bytes)) = found else {
        // Preserve the established permissive fallback for missing assets;
        // it remains explicit in runtime diagnostics rather than clearing UI.
        ctx.borrow_mut().unsupported("Objectives.load");
        return Ok(());
    };
    let parsed = super::objectives_file::parse(&bytes)
        .map_err(|e| mlua::Error::runtime(format!("Objectives.load {file}: {e}")))?;
    let count = u32::try_from(parsed.goals.len()).map_err(|_| mlua::Error::runtime("too many objectives"))?;
    let mut c = ctx.borrow_mut();
    let end = c
        .next_objective_id
        .checked_add(count)
        .ok_or_else(|| mlua::Error::runtime("objective ids exhausted"))?;
    let goals = parsed
        .goals
        .into_iter()
        .enumerate()
        .map(|(i, title)| Objective {
            id: c.next_objective_id + i as u32 + 1,
            title,
            description: String::new(),
            state: if !reset_completed
                && c.objectives
                    .get(i)
                    .is_some_and(|old| old.state == ObjectiveState::Complete)
            {
                ObjectiveState::Complete
            } else {
                ObjectiveState::Active
            },
        })
        .collect();
    c.objectives = goals;
    c.next_objective_id = end;
    c.objective_briefing = Some(ObjectiveBriefing {
        file,
        title: parsed.title,
        text: parsed.briefing,
        notes: parsed.notes,
    });
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod reload_policy_tests;
