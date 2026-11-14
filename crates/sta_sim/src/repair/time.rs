//! A2 modified per-system repair time over explicitly resolved inputs.
//!
//! The multiplier and inclusive crew-status rules are supported by controlled
//! original-instruction fixtures. This fixed-point policy has no live caller;
//! native float rounding and complete yard repair remain unmeasured.

use crate::Fixed;

/// Required race factors for per-system repair time. The caller resolves race
/// identity and supplies authored or separately established effective values.
/// This type deliberately provides no defaults for missing race configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepairTimeFactors {
    /// `repairStrength`, scaling seconds per hit point rather than HP per second.
    pub strength: Fixed,
    /// `repairYellow`, replacing the normal crew-status factor of one.
    pub yellow: Fixed,
    /// `repairRed`, replacing either normal or yellow status.
    pub red: Fixed,
    /// Inclusive `crewYellowStatus` fraction.
    pub crew_yellow_status: Fixed,
    /// Inclusive `crewRedStatus` fraction; red takes precedence.
    pub crew_red_status: Fixed,
}

/// Modified seconds per system hit point, before applying elapsed repair time.
///
/// `crew_fraction` is the already-resolved current crew / capacity ratio. The
/// caller supplies one for a unit with no crew cost; capacity resolution and
/// division by zero are outside this helper. Ratios above one are capped at one.
/// At or below the yellow threshold its multiplier applies; at or below the red
/// threshold red replaces yellow, even if the thresholds overlap or are reversed.
/// The selected status factor is multiplied by race strength and then base time,
/// and divided by `hyper_repair` (one when absent).
///
/// Signed values and authored zero times are preserved as arithmetic inputs;
/// the future service caller owns their gameplay interpretation. Each multiply
/// rounds down to Q32 and must fit Fixed before the next operation. Division
/// truncates toward zero. A zero divisor or any unrepresentable intermediate or
/// result returns `None`, a STA64 checked-input contract rather than a claim
/// about malformed native configurations. No floating point or wrapping casts
/// participate in this policy.
pub fn a2_modified_system_time(
    seconds_per_hp: Fixed,
    crew_fraction: Fixed,
    factors: RepairTimeFactors,
    hyper_repair: Option<Fixed>,
) -> Option<Fixed> {
    let fraction = crew_fraction.min(Fixed::ONE);
    let mut status = Fixed::ONE;
    if fraction <= factors.crew_yellow_status {
        status = factors.yellow;
    }
    if fraction <= factors.crew_red_status {
        status = factors.red;
    }
    let racial_status = checked_mul(status, factors.strength)?;
    let time = checked_mul(racial_status, seconds_per_hp)?;
    let divisor = hyper_repair.unwrap_or(Fixed::ONE).to_bits();
    if divisor == 0 {
        return None;
    }
    let result = (i128::from(time.to_bits()) << 32) / i128::from(divisor);
    Some(Fixed::from_bits(i64::try_from(result).ok()?))
}

fn checked_mul(a: Fixed, b: Fixed) -> Option<Fixed> {
    let result = (i128::from(a.to_bits()) * i128::from(b.to_bits())) >> 32;
    Some(Fixed::from_bits(i64::try_from(result).ok()?))
}
