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
//! - Weapon with `classLabel = "replaceweapon"`: a [`ReplaceDef`].
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
    /// Signed `randomweight`; one when absent.
    pub weight: i32,
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

/// A replacement weapon, whether instant or manually activated.
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
    /// Additional native replacement controls.
    pub options: ReplaceOptions,
}

/// Replacement controls read by the simulation. Derived defaults preserve
/// existing synthetic definitions; parsed weapons use native loader defaults.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplaceOptions {
    pub keep_name: bool,
    pub keep_owner: bool,
    pub reset_crew: bool,
    pub reset_energy: bool,
    pub maintain_orders: bool,
    pub ignore_same_class: bool,
    pub ignore_tech_tree: bool,
    pub find_open_spot: bool,
    pub warp_in: bool,
    pub transfer_stats: bool,
    pub select_mode: i64,
    pub concurrent_children: usize,
    pub cycle: Tick,
    /// Dilithium, Latinum, Metal, Biomatter.
    pub cost: [Fixed; 4],
    pub crew_cost: Fixed,
    pub event: Option<String>,
    pub warp_speed: Fixed,
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

/// The replacement a weapon ODF describes, if it is one.
pub fn replace_def(odf: &sta_odf::Odf) -> Option<ReplaceDef> {
    if !odf.class_label().is_some_and(|c| c.trim().eq_ignore_ascii_case("replaceweapon")) {
        return None;
    }
    let instant = [flag(odf, "replacementInstantPlayer"), flag(odf, "replacementInstantAI"), flag(odf, "replacementInstantDerelict")];
    let mut entries = Vec::new();
    for i in 0.. {
        let mut classes = Vec::new();
        for j in 0.. {
            let Some(name) = odf.str(&format!("replacement{i}class{j}")).map(stem) else { break };
            let weight = odf.i64(&format!("replacement{i}class{j}randomweight")).unwrap_or(1) as i32;
            let mut offset = [Fixed::ZERO; 3];
            if let Some(text) = odf.str(&format!("replacement{i}class{j}position")) {
                for (slot, part) in offset.iter_mut().zip(text.split(|c: char| c == ',' || c.is_ascii_whitespace()).filter(|s| !s.is_empty())) {
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
    let mut options = ReplaceOptions {
        transfer_stats: true,
        warp_speed: Fixed::from_int(400),
        ..ReplaceOptions::default()
    };
    options.keep_name = flag(odf, "replacementKeepName");
    options.keep_owner = flag(odf, "replacementKeepOwner");
    options.reset_crew = flag(odf, "replacementResetCrew");
    options.reset_energy = flag(odf, "replacementResetSpecialEnergy");
    options.maintain_orders = flag(odf, "replacementMaintainOrders");
    options.ignore_same_class = flag(odf, "ignoreSameClass");
    options.ignore_tech_tree = flag(odf, "ignoreTechTree");
    options.find_open_spot = flag(odf, "replacementFindOpenSpot");
    options.warp_in = flag(odf, "warpIn");
    options.transfer_stats = odf.bool("transferStats").unwrap_or(true);
    options.select_mode = odf.i64("selectMode").unwrap_or(0);
    options.concurrent_children = odf.i64("concurrentChilds").unwrap_or(0).max(0) as usize;
    options.cycle = sta_core::seconds_to_ticks(odf.f32("replacementInstantCycle").filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0) as f64);
    options.cost = ["DilithiumCost", "LatinumCost", "MetalCost", "BiomatterCost"]
        .map(|key| Fixed::from_int(odf.i64(key).unwrap_or(0) as i32));
    options.crew_cost = Fixed::from_int(odf.i64("CrewCost").unwrap_or(0) as i32);
    options.event = odf.str("replacementEvent").map(str::to_owned);
    options.warp_speed = Fixed::from_f64(odf.f32("warpInSpeed").filter(|v| v.is_finite()).unwrap_or(400.0) as f64);
    Some(ReplaceDef {
        instant,
        delay,
        mode: odf.i64("replacementMode").unwrap_or(0),
        required_officers: odf.i64("requiredOfficers"),
        entries,
        options,
    })
}

/// Final-build queue entries, in indexed order. The DLL probes every slot.
pub fn push_build_items(odf: &sta_odf::Odf) -> impl Iterator<Item = String> + '_ {
    (0..10).filter_map(|index| {
        let name = odf.str(&format!("pushBuildItem{index}"))?.trim();
        (!name.is_empty() && !name.eq_ignore_ascii_case("null")).then(|| stem(name))
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
        assert_eq!(r.entries[0].classes.iter().map(|c| (c.odf.as_str(), c.weight)).collect::<Vec<_>>(), [("a", 35), ("b", 1)]);
        assert_eq!(r.entries[1].classes[0].offset, [Fixed::from_int(-50), Fixed::ZERO, Fixed::from_int(100)]);
    }

    #[test]
    fn manual_replace_weapons_are_read() {
        let r = replace_def(&odf("classLabel = \"replaceweapon\"\nreplacement0class0 = \"a\"\n")).unwrap();
        assert_eq!(r.instant, [false; 3]);
        assert_eq!(r.entries[0].classes[0].weight, 1);
        assert!(replace_def(&odf("classLabel = \"cannon\"\nreplacementInstantAI = 1\n")).is_none());
    }

    #[test]
    fn replacement_options_and_signed_reuse_class_are_read() {
        let r = replace_def(&odf("classLabel = \"replaceweapon\"\nreplacement0class0 = \"\"\nreplacement0class0RandomWeight = -2\nreplacement0Class0Position = 1, 2 3\nreplacement0class1 = \"b\"\nreplacement0class1RandomWeight = 3\nreplacement0class3 = \"skipped\"\nreplacement2class0 = \"skipped\"\nreplacementKeepName = 1\nreplacementKeepOwner = 1\nreplacementResetCrew = 1\nreplacementResetSpecialEnergy = 1\nreplacementMaintainOrders = 1\nignoreSameClass = 1\nignoreTechTree = 1\nreplacementFindOpenSpot = 1\nwarpIn = 1\ntransferStats = 0\nselectMode = 2\nconcurrentChilds = 3\nreplacementInstantCycle = 0.5\nDilithiumCost = 10\nLatinumCost = 20\nMetalCost = 30\nBiomatterCost = 40\nCrewCost = 5\nreplacementEvent = \"swap\"\nwarpInSpeed = 500\n")).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].classes.len(), 2);
        assert_eq!((r.entries[0].classes[0].odf.as_str(), r.entries[0].classes[0].weight), ("", -2));
        assert_eq!(r.entries[0].classes[0].offset, [Fixed::ONE, Fixed::from_int(2), Fixed::from_int(3)]);
        assert_eq!(r.options, ReplaceOptions {
            keep_name: true, keep_owner: true, reset_crew: true, reset_energy: true,
            maintain_orders: true, ignore_same_class: true, ignore_tech_tree: true,
            find_open_spot: true, warp_in: true, transfer_stats: false,
            select_mode: 2, concurrent_children: 3, cycle: 15,
            cost: [10, 20, 30, 40].map(Fixed::from_int), crew_cost: Fixed::from_int(5),
            event: Some("swap".into()), warp_speed: Fixed::from_int(500),
        });
        let defaults = replace_def(&odf("classLabel = \"replaceweapon\"\n")).unwrap();
        assert!(defaults.entries.is_empty());
        assert!(defaults.options.transfer_stats);
        assert_eq!(defaults.options.warp_speed, Fixed::from_int(400));
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
