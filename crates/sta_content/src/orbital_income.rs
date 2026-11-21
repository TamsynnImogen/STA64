//! Typed content keys for orbital mining income (CON-51, part of #748).
//!
//! The Armada II family has stations of class `p_mining` that mine their
//! planet's resource node into the team's stock. The income rule itself is the
//! sim's; this module only reads the keys it needs, each as `None` or empty
//! where the key is absent (the executable's own defaults are globals that
//! were not read, so no value is invented). Grades: key names read from data;
//! which object reads which key read from the executable, static.
//!
//! - Station (`p_mining`): `miningMultiplier`, `resourcesCanHandle`.
//!   A station with a resolved `freighterName` also reads the resource mask.
//! - Resource node (the ODF a planet's `resource` names): `miningRate`,
//!   `trickleAmount`, `trickleMiningRateFraction`, `isInfinite`, `amt<slot>`.
//! - Race: `miningRateModifier` (see [`crate::Races::mining_rate_modifiers`]).
//!
//! Nothing in this crate applies them; what a node starts with and how it
//! mines is the sim's (SIM-72). `isInfinite` is the Armada II family's node key (read
//! from data); Armada I's inexhaustible moon carries `infinite` on the moon's own
//! ODF, which has no resource node, so it is not typed here.

use sta_core::Fixed;
use sta_odf::Odf;

use crate::resources::{KEYS, SLOTS};
use crate::{ResourceModel, Warning};

/// The station mining keys, including a free freighter's resource mask.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StationMining {
    /// `miningMultiplier`: the station's factor on the node's mining rate.
    /// `None` when absent (the engine copies it from a template class; not read).
    pub multiplier: Option<Fixed>,
    /// `resourcesCanHandle` as a mask over the four stock slots. `None` when
    /// the key is absent; an authored empty list is `Some` of all false.
    pub can_handle: Option<[bool; SLOTS]>,
}

impl StationMining {
    pub(crate) fn is_empty(&self) -> bool {
        *self == StationMining::default()
    }
}

/// The income keys of a planet's resource node. Every field is `None` when
/// its key is absent (engine defaults were not read).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeIncome {
    /// `miningRate`: units per second before the station and race factors.
    pub mining_rate: Option<Fixed>,
    /// `trickleAmount`: the level at which the node stops depleting.
    pub trickle_amount: Option<Fixed>,
    /// `trickleMiningRateFraction`: the share of `miningRate` yielded at the floor.
    pub trickle_fraction: Option<Fixed>,
    /// `isInfinite` of the node ODF: the node never depletes (`0` is `Some(false)`).
    pub infinite: Option<bool>,
    /// `amt<slot>` per slot: the quantity the ODF names. Not what a node
    /// starts with in the sim today.
    pub amounts: [Option<Fixed>; SLOTS],
}

impl NodeIncome {
    pub(crate) fn is_empty(&self) -> bool {
        *self == NodeIncome::default()
    }
}

/// A finite non-negative number, clamped so the fixed-point value cannot
/// overflow; `None` with a warning when malformed or negative.
pub(crate) fn read_nonneg(file: &str, odf: &Odf, key: &str, warnings: &mut Vec<Warning>) -> Option<Fixed> {
    let raw = odf.raw(key)?;
    match odf.f32(key) {
        Some(v) if v.is_finite() && v >= 0.0 => {
            let exact = v.to_string().parse::<f64>().unwrap_or(f64::from(v));
            Some(Fixed::from_f64(exact.min(1.0e9)))
        }
        _ => {
            warnings.push(Warning::new(file, format!("{key} = {raw} is not a finite non-negative number; ignored")));
            None
        }
    }
}

/// Station keys: the multiplier belongs to `p_mining`; a resolved free-unit
/// station may also declare the resources its first freighter should seek.
pub(crate) fn resolve_station(
    file: &str,
    odf: &Odf,
    class: &str,
    has_free_unit: bool,
    model: &ResourceModel,
    warnings: &mut Vec<Warning>,
) -> Option<StationMining> {
    if class != "p_mining" && !has_free_unit {
        return None;
    }
    let multiplier = (class == "p_mining").then(|| read_nonneg(file, odf, "miningMultiplier", warnings)).flatten();
    let can_handle = odf.raw("resourcesCanHandle").map(|raw| {
        let mut mask = [false; SLOTS];
        for word in raw.split_whitespace().map(|w| w.trim_matches('"')).filter(|w| !w.is_empty()) {
            let lower = word.to_ascii_lowercase();
            match KEYS.iter().position(|k| *k == lower).or_else(|| model.slot_of_text(word)) {
                Some(slot) => mask[slot] = true,
                None => warnings.push(Warning::new(file, format!("resourcesCanHandle word {word:?} names no resource; ignored"))),
            }
        }
        mask
    });
    let mining = StationMining { multiplier, can_handle };
    (!mining.is_empty()).then_some(mining)
}

/// The node keys; `None` when the node has none of them.
pub(crate) fn resolve_node(file: &str, node: &Odf, warnings: &mut Vec<Warning>) -> Option<NodeIncome> {
    let infinite = node.raw("isInfinite").and_then(|raw| {
        let flag = node.bool("isInfinite");
        if flag.is_none() {
            warnings.push(Warning::new(file, format!("isInfinite = {raw} is not a flag; ignored")));
        }
        flag
    });
    let mut amounts = [None; SLOTS];
    for (slot, amount) in amounts.iter_mut().enumerate() {
        *amount = read_nonneg(file, node, &crate::resources::amount_key(slot), warnings);
    }
    let income = NodeIncome {
        mining_rate: read_nonneg(file, node, "miningRate", warnings),
        trickle_amount: read_nonneg(file, node, "trickleAmount", warnings),
        trickle_fraction: read_nonneg(file, node, "trickleMiningRateFraction", warnings),
        infinite,
        amounts,
    };
    (!income.is_empty()).then_some(income)
}
