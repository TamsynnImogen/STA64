//! Synthetic coverage for the pure A2 dock-offset policy.

use sta_core::{Fixed, Vec3};
use sta_sim::repair::{DockBounds, a2_dock_offset};

fn v(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
        z: Fixed::from_int(z),
    }
}

fn bounds(min: Vec3, max: Vec3) -> DockBounds {
    DockBounds { min, max }
}

fn base_bounds() -> DockBounds {
    bounds(v(-2, -3, -4), v(5, 7, 11))
}

fn offset(side: i32, axis: Vec3) -> Option<Vec3> {
    a2_dock_offset(side, axis, base_bounds(), Fixed::ZERO, false)
}

fn assert_near(actual: Fixed, expected_raw: i128, tolerance: i128) {
    let delta = i128::from(actual.to_bits()) - expected_raw;
    assert!(
        delta.abs() <= tolerance,
        "actual={} expected_raw={expected_raw} delta={delta}",
        actual.to_bits()
    );
}

#[test]
fn cardinal_axis_selects_the_requested_bounds_faces() {
    let axis = v(0, 0, 1);
    assert_eq!(offset(0, axis), Some(v(0, 0, 0)));
    assert_eq!(offset(1, axis), Some(v(-5, 0, 0)));
    assert_eq!(offset(2, axis), Some(v(2, 0, 0)));
    assert_eq!(offset(3, axis), Some(v(2, 0, 0)));
    assert_eq!(offset(4, axis), Some(v(0, -7, 0)));
    assert_eq!(offset(8, axis), Some(v(0, 3, 0)));
    assert_eq!(offset(12, axis), Some(v(0, 3, 0)));
    assert_eq!(offset(15, axis), Some(v(2, 3, 0)));
    assert_eq!(offset(31, axis), Some(v(2, 3, 0)));
    assert_eq!(offset(-1, axis), Some(v(2, 3, 0)));
    assert_eq!(offset(-2, axis), Some(v(2, 3, 0)));
}

#[test]
fn cardinal_signs_flip_the_horizontal_offset() {
    assert_eq!(offset(1, v(0, 0, 1)), Some(v(-5, 0, 0)));
    assert_eq!(offset(1, v(0, 0, -1)), Some(v(5, 0, 0)));
    assert_eq!(offset(2, v(0, 0, 1)), Some(v(2, 0, 0)));
    assert_eq!(offset(2, v(0, 0, -1)), Some(v(-2, 0, 0)));
    assert_eq!(offset(1, v(1, 0, 0)), Some(v(0, 0, 5)));
    assert_eq!(offset(1, v(-1, 0, 0)), Some(v(0, 0, -5)));
}

#[test]
fn noncentered_bounds_are_used_as_given() {
    let noncentered = bounds(v(10, 20, 30), v(40, 60, 80));
    assert_eq!(
        a2_dock_offset(3, v(0, 0, 1), noncentered, Fixed::ZERO, false),
        Some(v(-10, 0, 0))
    );
    assert_eq!(
        a2_dock_offset(12, v(0, 0, 1), noncentered, Fixed::ZERO, false),
        Some(v(0, -20, 0))
    );
}

#[test]
fn radius_shift_subtracts_the_full_three_dimensional_axis() {
    let got = a2_dock_offset(
        10,
        v(3, 2, 4),
        base_bounds(),
        Fixed::from_int(5) / Fixed::from_int(4),
        true,
    )
    .unwrap();
    let scale = 1i128 << 32;
    // Expected values are the exact rational results in 32.32 raw units;
    // allow a few raw bits for integer normalization rounding.
    assert_near(got.x, -215 * scale / 100, 8);
    assert_near(got.y, scale / 2, 8);
    assert_near(got.z, -31 * scale / 5, 8);
}

#[test]
fn radius_shift_uses_radius_times_axis_without_normalizing_axis() {
    let no_shift = a2_dock_offset(0, v(0, 0, 1), base_bounds(), Fixed::ZERO, false).unwrap();
    let shifted = a2_dock_offset(
        0,
        v(0, 0, 1),
        base_bounds(),
        Fixed::from_int(5) / Fixed::from_int(4),
        true,
    )
    .unwrap();
    assert_eq!(no_shift, v(0, 0, 0));
    assert_eq!(shifted, Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: -Fixed::from_int(5) / Fixed::from_int(4) });

    let shifted_nonunit =
        a2_dock_offset(0, v(0, 2, 3), base_bounds(), Fixed::from_int(2), true).unwrap();
    assert_eq!(shifted_nonunit, v(0, -4, -6));
}

#[test]
fn horizontal_normalization_is_scale_invariant_without_radius_shift() {
    assert_eq!(offset(1, v(3, 0, 4)), offset(1, v(6, 0, 8)));
}

#[test]
fn diagonal_integer_axis_has_fixed_point_unit_perpendicular() {
    let actual = offset(
        1,
        Vec3 {
            x: Fixed::from_bits(1),
            y: Fixed::ZERO,
            z: Fixed::from_bits(1),
        },
    )
    .unwrap();
    assert_near(actual.x, -5i128 * 3_037_000_499, 2);
    assert_near(actual.z, 5i128 * 3_037_000_499, 2);
}

#[test]
fn zero_horizontal_axis_is_rejected_only_when_horizontal_offset_is_requested() {
    assert_eq!(offset(1, v(0, 1, 0)), None);
    assert_eq!(offset(2, Vec3::ZERO), None);
    assert_eq!(offset(0, Vec3::ZERO), Some(Vec3::ZERO));
    assert_eq!(offset(4, Vec3::ZERO), Some(v(0, -7, 0)));
}

#[test]
fn full_range_axis_is_supported_without_radius_shift() {
    let axis = Vec3 {
        x: Fixed::from_bits(i64::MIN),
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    };
    assert_eq!(offset(1, axis), Some(v(0, 0, -5)));
    let diagonal = offset(1, Vec3 { z: Fixed::from_bits(i64::MIN), ..axis }).unwrap();
    assert_near(diagonal.x, 5i128 * 3_037_000_499, 2);
    assert_near(diagonal.z, -5i128 * 3_037_000_499, 2);
}

#[test]
fn overflowing_radius_shift_returns_none() {
    let axis = Vec3 {
        x: Fixed::from_bits(i64::MAX),
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    };
    assert_eq!(
        a2_dock_offset(0, axis, base_bounds(), Fixed::from_bits(i64::MAX), true),
        None
    );
}

#[test]
fn wide_intermediate_cancellation_can_fit_final_fixed_result() {
    let extreme = bounds(
        Vec3 {
            x: Fixed::ZERO,
            y: Fixed::from_bits(i64::MIN),
            z: Fixed::ZERO,
        },
        Vec3::ZERO,
    );
    let axis = v(0, 1, 0);
    assert_eq!(
        a2_dock_offset(8, axis, extreme, Fixed::from_bits(1), true),
        Some(Vec3 {
            x: Fixed::ZERO,
            y: Fixed::from_bits(i64::MAX),
            z: Fixed::ZERO,
        })
    );
    assert_eq!(a2_dock_offset(8, axis, extreme, Fixed::ZERO, false), None);
}
