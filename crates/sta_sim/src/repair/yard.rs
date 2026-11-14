//! Pure A2 repair-yard choice over candidates already deemed eligible.
//!
//! The rule follows a controlled original-instruction reference: compare
//! surface distance / ship speed + 5 * queue length. The native float
//! near-tie rounding and live A2/Roots behavior have not been measured.

use crate::{EntityId, Fixed};

/// A yard supplied by the caller in native traversal order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YardCandidate {
    pub id: EntityId,
    /// Center distance minus both radii; negative values are retained.
    pub surface_distance: Fixed,
    /// Repair queue length used by the reference score.
    pub queued: u32,
}

/// Select the lowest-scoring eligible yard, preserving the first exact tie.
///
/// `speed` is expected to be the same positive ship speed for every candidate.
/// Empty candidates or a nonpositive speed produce no selection.
/// This validation is STA64's input contract; malformed native speed inputs
/// are outside the measured rule.
pub fn select_a2_yard(candidates: &[YardCandidate], speed: Fixed) -> Option<EntityId> {
    if speed <= Fixed::ZERO {
        return None;
    }

    let speed_bits = speed.to_bits() as i128;
    let mut best: Option<(EntityId, i128)> = None;

    for candidate in candidates {
        // Multiplying the reference score by positive speed preserves its
        // ordering while avoiding division and Fixed's wrapping operations.
        // Even the full-range product is below 2^98, within i128.
        let score = candidate.surface_distance.to_bits() as i128
            + speed_bits * 5 * candidate.queued as i128;
        if match best {
            None => true,
            Some((_, best_score)) => score < best_score,
        } {
            best = Some((candidate.id, score));
        }
    }

    best.map(|(id, _)| id)
}
