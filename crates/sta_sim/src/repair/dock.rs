//! A2 dock-offset math over already-resolved bounds and direction.
//!
//! Side-mask behavior is supported by controlled original-instruction fixtures.
//! This fixed-point port does not resolve SOD geometry or world/model transforms;
//! live A2/Roots docking and native float rounding remain unmeasured.

use crate::{Fixed, Vec3};

/// The bounds supplied to the reference offset calculation.
/// The caller owns geometry resolution and the coordinate-space convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockBounds {
    pub min: Vec3,
    pub max: Vec3,
}

/// Compute the craft offset selected by an authored `repairSide` mask.
///
/// Horizontal bits select the min (2) or max (1) extent along the perpendicular
/// to the direction in XZ. Vertical bits subtract the min (8) or max (4) bound.
/// Bits 2 and 8 take precedence when both alternatives are present; other bits
/// do not affect this helper. The optional radius shift uses the full direction,
/// including Y, without normalizing it.
///
/// No live order or profile calls this yet. Missing horizontal direction when
/// required, or an unrepresentable final result, returns `None` as a STA64 input
/// contract; malformed native inputs are outside the measured rule.
pub fn a2_dock_offset(
    side: i32,
    axis: Vec3,
    bounds: DockBounds,
    radius: Fixed,
    shift_by_radius: bool,
) -> Option<Vec3> {
    let mut offset = [0i128; 3];
    if side & 3 != 0 {
        let (nx, nz) = horizontal_perpendicular(axis)?;
        let extent = if side & 2 != 0 { bounds.min.x } else { bounds.max.x };
        offset[0] = (nx * i128::from(extent.to_bits())) >> 32;
        offset[2] = (nz * i128::from(extent.to_bits())) >> 32;
    }
    if side & 8 != 0 {
        offset[1] = -i128::from(bounds.min.y.to_bits());
    } else if side & 4 != 0 {
        offset[1] = -i128::from(bounds.max.y.to_bits());
    }
    if shift_by_radius {
        let radius = i128::from(radius.to_bits());
        for (value, component) in offset.iter_mut().zip([axis.x, axis.y, axis.z]) {
            *value -= (radius * i128::from(component.to_bits())) >> 32;
        }
    }
    // Keep wide intermediates: a radius shift can bring an initially out-of-
    // range bound offset back into Fixed's range. Never use wrapping casts.
    Some(Vec3 {
        x: Fixed::from_bits(i64::try_from(offset[0]).ok()?),
        y: Fixed::from_bits(i64::try_from(offset[1]).ok()?),
        z: Fixed::from_bits(i64::try_from(offset[2]).ok()?),
    })
}

/// Normalized horizontal perpendicular, as wide Q32 components.
fn horizontal_perpendicular(axis: Vec3) -> Option<(i128, i128)> {
    let x = i128::from(axis.x.to_bits());
    let z = i128::from(axis.z.to_bits());
    let largest = x.unsigned_abs().max(z.unsigned_abs());
    if largest == 0 {
        return None;
    }
    // Scale small directions up before sqrt, so a tiny diagonal does not lose
    // its length to integer rounding. Each magnitude stays <= 2^63; their
    // squared sum is <= 2^127 and fits u128, including two i64::MIN inputs.
    let shift = 63u32.saturating_sub(128 - largest.leading_zeros());
    let (x, z) = (x << shift, z << shift);
    let (ax, az) = (x.unsigned_abs(), z.unsigned_abs());
    let length = (ax * ax + az * az).isqrt() as i128;
    Some(((-z << 32) / length, (x << 32) / length))
}
