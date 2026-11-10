//! Parser for the plain-text mission objectives files.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedObjectives {
    pub title: String,
    pub briefing: String,
    pub notes: String,
    pub goals: Vec<String>,
}

/// Parses a UTF-8 objectives file, accepting an optional UTF-8 BOM.
pub(crate) fn parse(bytes: &[u8]) -> Result<ParsedObjectives, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "objectives file is not valid UTF-8".to_owned())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.is_empty() {
        return Err("objectives file is empty".to_owned());
    }

    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let Some(title_index) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return Err("objectives file has no mission heading".to_owned());
    };
    let title = lines[title_index].trim().to_owned();
    if title.starts_with('*') {
        return Err("objectives file requires a heading before its goals".to_owned());
    }

    let is_goal = |line: &str| line.trim().starts_with('*');
    let mut goals = Vec::new();
    let mut goal_indices = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(title_index + 1) {
        let trimmed = line.trim();
        if let Some(goal) = trimmed.strip_prefix('*') {
            let goal = goal.trim();
            if goal.is_empty() {
                return Err(format!(
                    "objectives file has an empty goal on line {}",
                    index + 1
                ));
            }
            goals.push(goal.to_owned());
            goal_indices.push(index);
        }
    }

    let briefing_end = goal_indices.first().copied().unwrap_or(lines.len());
    let briefing = metadata(&lines[title_index + 1..briefing_end]);
    let notes = if let Some(first_goal) = goal_indices.first().copied() {
        metadata(
            &lines[first_goal + 1..]
                .iter()
                .copied()
                .filter(|line| !is_goal(line))
                .collect::<Vec<_>>(),
        )
    } else {
        String::new()
    };

    Ok(ParsedObjectives {
        title,
        briefing,
        notes,
        goals,
    })
}

/// Removes only blank boundary lines; interior text and paragraph breaks remain.
fn metadata(lines: &[&str]) -> String {
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(start, |i| i + 1);
    lines[start..end].join("\n")
}

#[cfg(test)]
#[path = "objectives_file/tests.rs"]
mod tests;
