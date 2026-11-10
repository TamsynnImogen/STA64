//! Checked, deterministic geometry for map confinement (SIM-22 stage 1).
//!
//! Callers supply resolved rules; this module supplies no gameplay defaults.
//! Entity initialization, exemptions, state and movement integration are
//! separate policies. Merely constructing a volume changes no simulation state.

use sta_core::{Fixed, Vec3};

/// Why a confinement volume cannot be represented safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryError {
    NegativeInset,
    NegativeUpperEpsilon,
    ReversedBounds,
    DerivedLimitOverflow,
    InsufficientSpan,
}

impl std::fmt::Display for BoundaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NegativeInset => "map confinement inset must be nonnegative",
            Self::NegativeUpperEpsilon => "map confinement upper epsilon must be nonnegative",
            Self::ReversedBounds => "map confinement bounds are reversed",
            Self::DerivedLimitOverflow => "map confinement limits exceed Fixed range",
            Self::InsufficientSpan => "map confinement inset and epsilon do not fit the bounds",
        })
    }
}

impl std::error::Error for BoundaryError {}

/// A point and its outside reports. Inspection preserves the supplied position;
/// enforcement projects it. Vertical violations alone leave `outside_xz` false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryCheck {
    pub position: Vec3,
    pub outside: bool,
    pub outside_xz: bool,
}

/// Validated comparison and projection limits. X/Z upper projection can be
/// below the upper comparison limit; equality at the latter remains inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryVolume {
    lower: Vec3,
    upper: Vec3,
    upper_projection: Vec3,
}

impl BoundaryVolume {
    /// Inset X/Z by `inset`, then subtract `upper_epsilon` from their upper
    /// projection values only. Y retains the original bounds, including flat Y.
    ///
    /// Reject negative rules, reversed bounds, derived overflow or a span too
    /// small to contain the upper projection. No wrapping Fixed arithmetic is
    /// used to construct the limits. Unsupported tiny/invalid volumes are an
    /// explicit STA64 rejection policy, not inferred legacy invalid-map behavior.
    pub fn new(
        min: Vec3,
        max: Vec3,
        inset: Fixed,
        upper_epsilon: Fixed,
    ) -> Result<Self, BoundaryError> {
        if inset < Fixed::ZERO {
            return Err(BoundaryError::NegativeInset);
        }
        if upper_epsilon < Fixed::ZERO {
            return Err(BoundaryError::NegativeUpperEpsilon);
        }
        if min.x > max.x || min.y > max.y || min.z > max.z {
            return Err(BoundaryError::ReversedBounds);
        }
        let axis = |lo: Fixed, hi: Fixed| -> Result<_, BoundaryError> {
            let lower = lo
                .to_bits()
                .checked_add(inset.to_bits())
                .ok_or(BoundaryError::DerivedLimitOverflow)?;
            let upper = hi
                .to_bits()
                .checked_sub(inset.to_bits())
                .ok_or(BoundaryError::DerivedLimitOverflow)?;
            let projection = upper
                .checked_sub(upper_epsilon.to_bits())
                .ok_or(BoundaryError::DerivedLimitOverflow)?;
            if lower > projection {
                return Err(BoundaryError::InsufficientSpan);
            }
            Ok((
                Fixed::from_bits(lower),
                Fixed::from_bits(upper),
                Fixed::from_bits(projection),
            ))
        };
        let (lx, ux, px) = axis(min.x, max.x)?;
        let (lz, uz, pz) = axis(min.z, max.z)?;
        Ok(Self {
            lower: Vec3 {
                x: lx,
                y: min.y,
                z: lz,
            },
            upper: Vec3 {
                x: ux,
                y: max.y,
                z: uz,
            },
            upper_projection: Vec3 {
                x: px,
                y: max.y,
                z: pz,
            },
        })
    }

    /// Report strict violations on every axis. With `enforce`, project lower
    /// violations to the lower limit and upper violations to the upper
    /// projection value. Inspection returns the original point unchanged.
    /// Valid enforced results are inside on reinspection and are idempotent.
    pub fn check(&self, position: Vec3, enforce: bool) -> BoundaryCheck {
        let axis = |value: Fixed, lo: Fixed, hi: Fixed, projection: Fixed| {
            if value < lo {
                (if enforce { lo } else { value }, true)
            } else if value > hi {
                (if enforce { projection } else { value }, true)
            } else {
                (value, false)
            }
        };
        let (x, ox) = axis(
            position.x,
            self.lower.x,
            self.upper.x,
            self.upper_projection.x,
        );
        let (y, oy) = axis(
            position.y,
            self.lower.y,
            self.upper.y,
            self.upper_projection.y,
        );
        let (z, oz) = axis(
            position.z,
            self.lower.z,
            self.upper.z,
            self.upper_projection.z,
        );
        BoundaryCheck {
            position: Vec3 { x, y, z },
            outside: ox || oy || oz,
            outside_xz: ox || oz,
        }
    }
}

#[cfg(test)]
mod tests;
