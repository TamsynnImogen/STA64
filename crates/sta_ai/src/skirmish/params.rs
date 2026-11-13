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
//! | `max_attack_force` | ceiling of the wave size (waves are drawn between the minimum and it) |
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
        p.overlay(aip);
        // An AIP's `max_attack_force 1` beside a larger minimum is a typo
        // the game tolerates; so do we.
        p.max_attack = p.max_attack.max(p.min_attack);
        p
    }

    /// Load `aip` over these values, as the original's AIP switch does (AI-06;
    /// read from the executable, static): a key the file sets (and that is in
    /// range) replaces the value, a key it leaves out keeps the value, and the
    /// build list is replaced only when the file has `the_build_list`.
    pub fn overlay(&mut self, aip: &Aip) {
        let count = |key: &str, lo: i64, hi: i64| aip.int(key).filter(|v| (lo..=hi).contains(v)).map(|v| v as usize);
        if let Some(v) = count("min_resource_force", 1, MINERS_MAX) {
            self.miners = v;
        }
        if let Some(v) = count("min_defense_force", 0, 64) {
            self.defence = v;
        }
        if let Some(v) = count("min_attack_force", 1, 64) {
            self.min_attack = v;
        }
        if let Some(v) = count("max_attack_force", 1, 64) {
            self.max_attack = v;
        }
        if let Some(list) = aip.build_list("the_build_list") {
            self.build_list = list;
        }
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

    #[test]
    fn overlay_keeps_what_the_file_leaves_out() {
        let (first, _) = Aip::parse("a.aip", "int min_resource_force = 5;\nint min_attack_force = 6;\n");
        let mut p = Params::from_aip(&first);
        let (second, _) = Aip::parse("b.aip", "int min_attack_force = 9;\n");
        p.overlay(&second);
        assert_eq!((p.miners, p.min_attack), (5, 9));
        // The stored maximum is the file's, not a clamp of an earlier minimum.
        let (a, _) = Aip::parse("a.aip", "int min_attack_force = 20;\n");
        let (b, _) = Aip::parse("b.aip", "int min_attack_force = 4;\n");
        let mut p = Params::default();
        p.overlay(&a);
        p.overlay(&b);
        assert_eq!((p.min_attack, p.max_attack), (4, DEFAULT_MAX_ATTACK));
    }
}
