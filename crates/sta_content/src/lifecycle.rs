//! Object lifecycle keys (SIM-28): how long a helper or effect lives and what
//! an instant `replaceweapon` turns it into.
//!
//! Read from the ODFs of Fleet Operations' start helpers (`all_ai_pod`,
//! `all_start`, `8472start`, `<race>_effectsS`) and the effects they spawn
//! (`effect_rm`, `u_ball_obj`): the sim reads these typed values, never the
//! ODFs, and the values are in the content hash.
//!
//! - Unit: `hidden`, `invalidAsTarget`, and a lifetime in seconds from
//!   `autoExpireTime` (else `lifeTimer`).
//! - Weapon with `classLabel = "replaceweapon"` and at least one of
//!   `replacementInstantPlayer` / `AI` / `Derelict` set: a [`ReplaceDef`].
//!   `replacement<i>class<j>` is class `j` of entry `i`, `replacement<i>condition`
//!   the source ODF the entry applies to, `replacement<i>class<j>randomweight`
//!   its weight and `replacement<i>Class<j>Position` an `x,y,z` offset.

use sta_core::{Fixed, Tick};

/// Which kind of owner an instant replacement fires for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// A human, remote or idle seat.
    Player,
    /// A computer seat.
    Ai,
    /// No owner.
    Derelict,
}

/// One class an entry can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceClass {
    /// ODF stem.
    pub odf: String,
    /// `randomweight`; zero when absent.
    pub weight: u32,
    /// Offset from the source, world axes.
    pub offset: [Fixed; 3],
}

/// `replacement<i>*`: a condition and the classes it can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceEntry {
    /// ODF stem of the source this entry applies to; `None`: any.
    pub condition: Option<String>,
    pub classes: Vec<ReplaceClass>,
}

/// An instant replacement weapon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceDef {
    /// Fires for a player, AI or derelict owner (`replacementInstant*`).
    pub instant: [bool; 3],
    /// `replacementInstantDelay`, ticks after the source appears.
    pub delay: Tick,
    /// `replacementMode`: 0 spawns every matching entry, 2 the first.
    pub mode: i64,
    /// `requiredOfficers`: the owner's officer maximum must reach it.
    pub required_officers: Option<i64>,
    pub entries: Vec<ReplaceEntry>,
}

impl ReplaceDef {
    /// Whether the weapon fires for `owner`.
    pub fn fires_for(&self, owner: Owner) -> bool {
        self.instant[owner as usize]
    }
}

fn stem(name: &str) -> String {
    crate::odf::stem(name)
}

fn flag(odf: &sta_odf::Odf, key: &str) -> bool {
    odf.i64(key).is_some_and(|v| v != 0)
}

/// The instant replacement a weapon ODF describes, if it is one.
pub fn replace_def(odf: &sta_odf::Odf) -> Option<ReplaceDef> {
    if !odf.class_label().is_some_and(|c| c.trim().eq_ignore_ascii_case("replaceweapon")) {
        return None;
    }
    let instant = [flag(odf, "replacementInstantPlayer"), flag(odf, "replacementInstantAI"), flag(odf, "replacementInstantDerelict")];
    if !instant.iter().any(|&b| b) {
        return None;
    }
    let mut entries = Vec::new();
    for i in 0..64 {
        let mut classes = Vec::new();
        for j in 0..64 {
            let Some(name) = odf.str(&format!("replacement{i}class{j}")).map(stem).filter(|n| !n.is_empty()) else { break };
            let weight = odf.i64(&format!("replacement{i}class{j}randomweight")).unwrap_or(0).clamp(0, 1_000_000) as u32;
            let mut offset = [Fixed::ZERO; 3];
            if let Some(text) = odf.str(&format!("replacement{i}class{j}position")) {
                for (slot, part) in offset.iter_mut().zip(text.split(',')) {
                    if let Ok(v) = part.trim().parse::<f64>()
                        && v.is_finite()
                    {
                        *slot = Fixed::from_f64(v.clamp(-1e6, 1e6));
                    }
                }
            }
            classes.push(ReplaceClass { odf: name, weight, offset });
        }
        if classes.is_empty() {
            break;
        }
        let condition = odf.str(&format!("replacement{i}condition")).map(stem).filter(|c| !c.is_empty());
        entries.push(ReplaceEntry { condition, classes });
    }
    let delay = sta_core::seconds_to_ticks(odf.f32("replacementInstantDelay").filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0) as f64);
    Some(ReplaceDef {
        instant,
        delay,
        mode: odf.i64("replacementMode").unwrap_or(0),
        required_officers: odf.i64("requiredOfficers"),
        entries,
    })
}

/// `hidden = 1`.
pub fn hidden(odf: &sta_odf::Odf) -> bool {
    flag(odf, "hidden")
}

/// `invalidAsTarget = 1`.
pub fn invalid_as_target(odf: &sta_odf::Odf) -> bool {
    flag(odf, "invalidAsTarget")
}

/// Lifetime in ticks: `autoExpireTime`, else `lifeTimer`; `None` when absent or not positive.
pub fn life_ticks(odf: &sta_odf::Odf) -> Option<Tick> {
    let secs = ["autoExpireTime", "lifeTimer"].iter().filter_map(|k| odf.f32(k).filter(|v| v.is_finite() && *v > 0.0)).next()?;
    Some(sta_core::seconds_to_ticks(secs as f64).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn odf(text: &str) -> sta_odf::Odf {
        sta_odf::resolve("w", &mut |n| (n.trim_end_matches(".odf") == "w").then(|| text.to_owned())).unwrap()
    }

    #[test]
    fn instant_replace_weapons_are_read_with_conditions_weights_and_offsets() {
        let w = odf("classLabel = \"replaceweapon\"\nreplacementInstantAI = 1\nreplacementInstantDelay = 0.2\nreplacementMode = 2\n\
                     replacement0condition = \"all_ai_pod\"\nreplacement0class0 = \"a.odf\"\nreplacement0class0randomweight = 35\n\
                     replacement0class1 = \"b\"\nreplacement1class0 = \"c\"\nreplacement1Class0Position = -50,0,100\n");
        let r = replace_def(&w).unwrap();
        assert!(r.fires_for(Owner::Ai) && !r.fires_for(Owner::Player) && !r.fires_for(Owner::Derelict));
        assert_eq!((r.delay, r.mode, r.required_officers), (6, 2, None));
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].condition.as_deref(), Some("all_ai_pod"));
        assert_eq!(r.entries[0].classes.iter().map(|c| (c.odf.as_str(), c.weight)).collect::<Vec<_>>(), [("a", 35), ("b", 0)]);
        assert_eq!(r.entries[1].classes[0].offset, [Fixed::from_int(-50), Fixed::ZERO, Fixed::from_int(100)]);
    }

    #[test]
    fn only_instant_replace_weapons_count() {
        assert!(replace_def(&odf("classLabel = \"replaceweapon\"\nreplacement0class0 = \"a\"\n")).is_none());
        assert!(replace_def(&odf("classLabel = \"cannon\"\nreplacementInstantAI = 1\n")).is_none());
    }

    #[test]
    fn lifetime_prefers_auto_expire_and_ignores_zero() {
        assert_eq!(life_ticks(&odf("autoExpireTime = 0.5\nlifeTimer = 9\n")), Some(15));
        assert_eq!(life_ticks(&odf("lifeTimer = 1\n")), Some(30));
        assert_eq!(life_ticks(&odf("lifeTimer = 0\n")), None);
        assert_eq!(life_ticks(&odf("autoExpireTime = 0\nlifeTimer = 1\n")), Some(30));
        assert_eq!(life_ticks(&odf("maxHealth = 1\n")), None);
    }
}
