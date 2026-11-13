//! A mission that ends in a loss after reaching calls STA64 cannot do is *blocked*, not lost.
//!
//! A script's lose condition reads the world the script itself changed (a label it expected, a crew
//! count, an entity it just added). When an edit or a name the script used is unbound or counted
//! unsupported (`MissionRuntime::unsupported`), the world differs from the one the script was
//! written for, and a loss it then reaches says nothing about the player. The rule STA64 follows:
//! a loss reached after any unbound or unsupported use is reported as blocked, with the names, and
//! changes no campaign progress; a win is always a win. Evidence grade: a rule of STA64, chosen so a
//! gap cannot turn into a lost mission; it is conservative (a genuine loss after an unrelated gap is
//! also reported as blocked) until the gaps close (WORKPLAN SIM-54..61).
//!
//! The Armada II path has the same principle for its own end calls (LUA-13, `api/a2/mission_end.rs`);
//! both should end in one outcome type, `Won | Lost | Blocked(reasons)`, with this list as the
//! shared reason source.

use crate::runtime::MissionRuntime;

/// How many names a reason list shows.
pub const MAX_NAMES: usize = 6;

/// The unbound and unsupported names the script has used so far, most used first.
///
/// Also: a script error (a call that raised: a binding STA64 lacks, an overload it does not
/// implement, content it does not have) and an end STA64 could not observe, since neither says
/// anything about the player either.
pub fn gaps(runtime: &MissionRuntime) -> Vec<String> {
    let mut v = gaps_of(&runtime.unsupported());
    let mut errors: Vec<String> = Vec::new();
    for e in runtime.errors() {
        let line = e.lines().next().unwrap_or("");
        // The edits the sim rejected are counted above.
        if line.contains("simulation rejected") {
            continue;
        }
        let line = format!("script error: {}", line.chars().take(90).collect::<String>());
        if !errors.contains(&line) {
            errors.push(line);
        }
    }
    v.extend(errors);
    if runtime.finished() && runtime.eval::<bool>("return rawget(_G, '__sta64_outcome') == nil").unwrap_or(false) {
        v.push("end reached through a reference the host could not observe".to_owned());
    }
    v
}

/// [`gaps`] over a counted list.
pub fn gaps_of(counted: &[(String, u32)]) -> Vec<String> {
    let mut v: Vec<&(String, u32)> = counted.iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.into_iter().map(|(k, n)| format!("{k} x{n}")).collect()
}

/// Whether a finished mission is blocked: it was lost (`won` false) with gaps in its run.
pub fn is_blocked(won: bool, gaps: &[String]) -> bool {
    !won && !gaps.is_empty()
}

/// One line for the player: the first few names.
pub fn summary(gaps: &[String]) -> String {
    let shown: Vec<&str> = gaps.iter().take(MAX_NAMES).map(String::as_str).collect();
    let more = gaps.len().saturating_sub(MAX_NAMES);
    if more > 0 { format!("{} (+{more} more)", shown.join("; ")) } else { shown.join("; ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loss_after_a_gap_is_blocked_and_a_win_or_a_clean_loss_is_not() {
        let g = gaps_of(&[("Entity.shields = x".into(), 2), ("stub global rel".into(), 7)]);
        assert_eq!(g, ["stub global rel x7", "Entity.shields = x x2"]);
        assert!(is_blocked(false, &g));
        assert!(!is_blocked(true, &g), "a win is a win");
        assert!(!is_blocked(false, &[]), "a loss with nothing unsupported is a real loss");
    }

    #[test]
    fn a_script_error_and_an_unobserved_end_are_gaps_too() {
        let rt = crate::runtime::MissionRuntime::from_source("e.lua", "MMM.register(function() error('boom') end)", 1);
        let mut rt = rt;
        rt.step(1);
        let g = gaps(&rt);
        assert!(g.iter().any(|x| x.starts_with("script error: ") && x.contains("boom")), "{g:?}");
        assert!(is_blocked(false, &g));
    }

    #[test]
    fn the_summary_is_short() {
        let g: Vec<String> = (0..9).map(|i| format!("n{i} x1")).collect();
        assert!(summary(&g).ends_with("(+3 more)"));
        assert_eq!(summary(&g[..2]), "n0 x1; n1 x1");
    }
}
