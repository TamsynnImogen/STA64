//! What the skirmish AI is tuned by: AIP values when a file is given,
//! defaults otherwise (CON-14, `docs/data/aip.md`).
//!
//! The installed AIPs carry far more than a controller this size can honour
//! (evaluation priorities, danger modifiers, scheduler periods). Only the
//! values that map onto its four jobs are read:
//!
//! | AIP key | Used as |
//! |---|---|
//! | `min_resource_force` | miners to keep |
//! | `min_defense_force` | warships held back at home |
//! | `min_attack_force` | size of the first wave |
//! | `max_attack_force` | largest wave |
//! | `the_build_list` | standing build requests, in order |
//!
//! A key that is missing or out of range falls back to the default, so a
//! half-written AIP still plays.

use sta_content::{Aip, BuildItem};

/// Miners kept without an AIP.
pub const DEFAULT_MINERS: usize = 3;
/// Warships held at home without an AIP.
pub const DEFAULT_DEFENCE: usize = 2;
/// Size of the first wave without an AIP.
pub const DEFAULT_MIN_ATTACK: usize = 4;
/// Largest wave without an AIP.
pub const DEFAULT_MAX_ATTACK: usize = 12;
/// Warships at which the AI stops building them.
pub const ARMY_CAP: usize = 60;
/// Most miners any AIP can ask for.
const MINERS_MAX: i64 = 12;

/// Tuning for one controller.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub miners: usize,
    pub defence: usize,
    pub min_attack: usize,
    pub max_attack: usize,
    /// Standing build requests (`the_build_list`), in order; empty without an AIP.
    pub build_list: Vec<BuildItem>,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            miners: DEFAULT_MINERS,
            defence: DEFAULT_DEFENCE,
            min_attack: DEFAULT_MIN_ATTACK,
            max_attack: DEFAULT_MAX_ATTACK,
            build_list: Vec::new(),
        }
    }
}

impl Params {
    /// Defaults overridden by whatever `aip` sets sensibly.
    pub fn from_aip(aip: &Aip) -> Params {
        let mut p = Params::default();
        let count = |key: &str, lo: i64, hi: i64| aip.int(key).filter(|v| (lo..=hi).contains(v)).map(|v| v as usize);
        if let Some(v) = count("min_resource_force", 1, MINERS_MAX) {
            p.miners = v;
        }
        if let Some(v) = count("min_defense_force", 0, 64) {
            p.defence = v;
        }
        if let Some(v) = count("min_attack_force", 1, 64) {
            p.min_attack = v;
        }
        if let Some(v) = count("max_attack_force", 1, 64) {
            p.max_attack = v;
        }
        // An AIP's `max_attack_force 1` beside a larger minimum is a typo
        // the game tolerates; so do we.
        p.max_attack = p.max_attack.max(p.min_attack);
        p.build_list = aip.build_list("the_build_list").unwrap_or_default();
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_without_an_aip() {
        assert_eq!(Params::default().miners, DEFAULT_MINERS);
        assert!(Params::default().build_list.is_empty());
    }

    #[test]
    fn aip_values_override_and_bad_ones_do_not() {
        let text = "int min_resource_force = 5;\nint min_defense_force = 0;\nint min_attack_force = 6;\n\
                    int max_attack_force = 2;\nint junk = 1;\n";
        let (aip, _) = Aip::parse("t.aip", text);
        let p = Params::from_aip(&aip);
        assert_eq!(p.miners, 5);
        assert_eq!(p.defence, 0);
        assert_eq!(p.min_attack, 6);
        assert_eq!(p.max_attack, 6, "max is raised to the minimum");
        let (aip, _) = Aip::parse("t.aip", "int min_resource_force = 0;\nint min_attack_force = -3;\n");
        let p = Params::from_aip(&aip);
        assert_eq!((p.miners, p.min_attack), (DEFAULT_MINERS, DEFAULT_MIN_ATTACK));
    }
}
