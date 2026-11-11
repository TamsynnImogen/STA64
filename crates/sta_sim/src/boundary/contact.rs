//! Checked XZ contacts for a confined actor and static solids.

use sta_core::{Fixed, Vec3};

use super::BoundaryVolume;
use crate::spatial::Solid;

pub(crate) type Distance = (bool, u128); // 129-bit sum of two squared raw-coordinate differences.

pub(crate) fn distance(a: Vec3, b: Vec3) -> Distance {
    let dx = (i128::from(a.x.to_bits()) - i128::from(b.x.to_bits())).unsigned_abs();
    let dz = (i128::from(a.z.to_bits()) - i128::from(b.z.to_bits())).unsigned_abs();
    let (low, carry) = (dx * dx).overflowing_add(dz * dz);
    (carry, low)
}

fn raw(value: Fixed) -> i128 {
    i128::from(value.to_bits())
}

fn clear(point: Vec3, solid: &Solid, radius: Fixed) -> bool {
    match *solid {
        Solid::Sphere {
            centre,
            radius: solid_radius,
        } => {
            if solid_radius < Fixed::ZERO {
                return false;
            }
            let Ok(sum) = u128::try_from(raw(solid_radius) + raw(radius)) else {
                return false;
            };
            let Some(square) = sum.checked_mul(sum) else {
                return false;
            };
            let (carry, low) = distance(point, centre);
            carry || low >= square
        }
        Solid::Box { min, max } => {
            if min.x > max.x || min.z > max.z {
                return false;
            }
            let radius = raw(radius);
            raw(point.x) <= raw(min.x) - radius
                || raw(point.x) >= raw(max.x) + radius
                || raw(point.z) <= raw(min.z) - radius
                || raw(point.z) >= raw(max.z) + radius
        }
    }
}

// Exact 128-by-128 product as high and low limbs. The sphere leg test compares
// cross² and radius² * length² without a rounded square root.
fn product(a: u128, b: u128) -> (u128, u128) {
    let mask = (1_u128 << 64) - 1;
    let (al, ah, bl, bh) = (a & mask, a >> 64, b & mask, b >> 64);
    let (m1, m2) = (ah * bl, al * bh);
    let (low, carry1) = (al * bl).overflowing_add(m1 << 64);
    let (low, carry2) = low.overflowing_add(m2 << 64);
    let high = ah * bh + (m1 >> 64) + (m2 >> 64) + u128::from(carry1) + u128::from(carry2);
    (high, low)
}

fn visible(origin: Vec3, target: Vec3, solid: &Solid, radius: Fixed) -> bool {
    // Recovery from a pre-existing overlap may reach a clear endpoint.
    if !clear(origin, solid, radius) {
        return true;
    }
    let (px, pz, qx, qz) = (raw(origin.x), raw(origin.z), raw(target.x), raw(target.z));
    let (dx, dz) = (qx - px, qz - pz);
    let checked = || -> Option<bool> {
        match *solid {
            Solid::Sphere {
                centre,
                radius: solid_radius,
            } => {
                let (cx, cz) = (raw(centre.x) - px, raw(centre.z) - pz);
                let length2 = dx.checked_mul(dx)?.checked_add(dz.checked_mul(dz)?)?;
                if length2 == 0 {
                    return Some(true);
                }
                let dot = cx.checked_mul(dx)?.checked_add(cz.checked_mul(dz)?)?;
                if dot <= 0 || dot >= length2 {
                    return Some(true); // Clear endpoints have already been checked.
                }
                let cross = cx
                    .checked_mul(dz)?
                    .checked_sub(cz.checked_mul(dx)?)?
                    .unsigned_abs();
                let sum = u128::try_from(raw(solid_radius) + raw(radius)).ok()?;
                Some(product(cross, cross) >= product(sum.checked_mul(sum)?, length2 as u128))
            }
            Solid::Box { min, max } => {
                type Fraction = (i128, i128); // positive denominator
                let compare = |a: Fraction, b: Fraction| {
                    Some(a.0.checked_mul(b.1)?.cmp(&b.0.checked_mul(a.1)?))
                };
                let (mut low, mut high): (Fraction, Fraction) = ((0, 1), (1, 1));
                for (at, step, lo, hi) in [
                    (px, dx, raw(min.x) - raw(radius), raw(max.x) + raw(radius)),
                    (pz, dz, raw(min.z) - raw(radius), raw(max.z) + raw(radius)),
                ] {
                    if step == 0 {
                        if at <= lo || at >= hi {
                            return Some(true);
                        }
                        continue;
                    }
                    let (entry, exit) = if step > 0 {
                        ((lo - at, step), (hi - at, step))
                    } else {
                        ((at - hi, -step), (at - lo, -step))
                    };
                    if compare(entry, low)?.is_gt() {
                        low = entry;
                    }
                    if compare(exit, high)?.is_lt() {
                        high = exit;
                    }
                    if !compare(low, high)?.is_lt() {
                        return Some(true);
                    }
                }
                Some(false)
            }
        }
    };
    checked().unwrap_or(false)
}

fn endpoint_clear(volume: &BoundaryVolume, target: Vec3, radius: Fixed, solids: &[Solid]) -> bool {
    !volume.check(target, false).outside && solids.iter().all(|solid| clear(target, solid, radius))
}

/// Whether a target is in the volume, physically clear and reached by a free
/// XZ leg. The origin itself need not be inside the volume or clear of solids.
pub(crate) fn legal_target(
    volume: &BoundaryVolume,
    origin: Vec3,
    target: Vec3,
    radius: Fixed,
    solids: &[Solid],
) -> bool {
    radius >= Fixed::ZERO
        && endpoint_clear(volume, target, radius, solids)
        && solids
            .iter()
            .all(|solid| visible(origin, target, solid, radius))
}

fn point(x: i128, z: i128, y: Fixed) -> Option<Vec3> {
    Some(Vec3 {
        x: Fixed::from_bits(i64::try_from(x).ok()?),
        y,
        z: Fixed::from_bits(i64::try_from(z).ok()?),
    })
}

fn candidates(
    volume: &BoundaryVolume,
    desired: Vec3,
    blocker: &Solid,
    expansion: Fixed,
) -> Vec<Vec3> {
    let mut out = Vec::new();
    let y = volume.check(desired, true).position.y;
    match *blocker {
        Solid::Sphere { centre, radius } => {
            if radius < Fixed::ZERO {
                return out;
            }
            let expanded = raw(radius) + raw(expansion);
            if !(0..=i128::from(i64::MAX)).contains(&expanded) {
                return out;
            }
            let (cx, cz) = (raw(centre.x), raw(centre.z));
            for (x, z) in [
                (cx - expanded, cz),
                (cx + expanded, cz),
                (cx, cz - expanded),
                (cx, cz + expanded),
            ] {
                if let Some(candidate) = point(x, z, y) {
                    out.push(candidate);
                }
            }
            let (lower, upper) = volume.comparison_limits();
            for (edge_x, edge_z) in [(lower.x, lower.z), (upper.x, upper.z)] {
                for (swapped, edge) in [(false, edge_x), (true, edge_z)] {
                    let delta = raw(edge) - if swapped { cz } else { cx };
                    if delta.unsigned_abs() > expanded as u128 {
                        continue;
                    }
                    let square = (expanded * expanded - delta * delta) as u128;
                    let floor = square.isqrt();
                    let ceil = floor + u128::from(floor * floor < square);
                    for sign in [-1_i128, 1] {
                        let offset = sign * ceil as i128;
                        let (x, z) = if swapped {
                            (cx + offset, raw(edge))
                        } else {
                            (raw(edge), cz + offset)
                        };
                        if let Some(candidate) = point(x, z, y) {
                            out.push(candidate);
                        }
                    }
                }
            }
        }
        Solid::Box { min, max } => {
            if min.x > max.x || min.z > max.z {
                return out;
            }
            let expansion = raw(expansion);
            let (lo_x, hi_x) = (raw(min.x) - expansion, raw(max.x) + expansion);
            let (lo_z, hi_z) = (raw(min.z) - expansion, raw(max.z) + expansion);
            let (x, z) = (
                raw(desired.x).clamp(lo_x, hi_x),
                raw(desired.z).clamp(lo_z, hi_z),
            );
            for (x, z) in [(lo_x, z), (hi_x, z), (x, lo_z), (x, hi_z)] {
                if let Some(candidate) = point(x, z, y) {
                    out.push(candidate);
                }
            }
        }
    }
    out
}

/// Find a checked contact around one blocker. Candidate expansion uses the
/// physical blocker size, actor radius and caller-supplied steering margin;
/// validation always uses physical clearance only.
pub(crate) fn contact(
    volume: &BoundaryVolume,
    origin: Vec3,
    desired: Vec3,
    blocker: &Solid,
    radius: Fixed,
    margin: Fixed,
    solids: &[Solid],
) -> Option<Vec3> {
    if radius < Fixed::ZERO || margin < Fixed::ZERO {
        return None;
    }
    let expansion = raw(radius) + raw(margin);
    if expansion > i128::from(i64::MAX) {
        return None;
    }
    candidates(volume, desired, blocker, Fixed::from_bits(expansion as i64))
        .into_iter()
        .enumerate()
        .filter(|(_, candidate)| {
            legal_target(volume, origin, *candidate, radius, solids)
                && clear(*candidate, blocker, radius)
                && visible(origin, *candidate, blocker, radius)
        })
        .min_by_key(|(index, candidate)| {
            (
                distance(*candidate, desired),
                distance(*candidate, origin),
                *index,
            )
        })
        .map(|(_, candidate)| candidate)
}

#[cfg(test)]
#[path = "contact_tests.rs"]
mod tests;
