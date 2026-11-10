use super::{BoundaryCheck, BoundaryError, BoundaryVolume};
use sta_core::{Fixed, Vec3};

const NATIVE_EPSILON_BITS: i64 = 429_496_736;

fn v(x: i64, y: i64, z: i64) -> Vec3 {
    Vec3 {
        x: Fixed::from_bits(x),
        y: Fixed::from_bits(y),
        z: Fixed::from_bits(z),
    }
}

fn reference() -> BoundaryVolume {
    BoundaryVolume::new(
        v(0, -100 * Fixed::ONE.to_bits(), 0),
        v(
            1000 * Fixed::ONE.to_bits(),
            100 * Fixed::ONE.to_bits(),
            1000 * Fixed::ONE.to_bits(),
        ),
        Fixed::from_int(100),
        Fixed::from_bits(NATIVE_EPSILON_BITS),
    )
    .unwrap()
}

#[test]
fn strict_edges_and_neighbour_bits_preserve_upper_comparison_equality() {
    let bounds = reference();
    let unit = Fixed::ONE.to_bits();
    let low = 100 * unit;
    let high = 900 * unit;
    let projected_high = high - NATIVE_EPSILON_BITS;
    let middle = 500 * unit;
    for (point, expected, outside) in [
        (v(low - 1, 0, middle), v(low, 0, middle), true),
        (v(low, 0, middle), v(low, 0, middle), false),
        (v(low + 1, 0, middle), v(low + 1, 0, middle), false),
        (v(high - 1, 0, middle), v(high - 1, 0, middle), false),
        (v(high, 0, middle), v(high, 0, middle), false),
        (v(high + 1, 0, middle), v(projected_high, 0, middle), true),
        (v(middle, 0, low - 1), v(middle, 0, low), true),
        (v(middle, 0, high + 1), v(middle, 0, projected_high), true),
    ] {
        assert_eq!(
            bounds.check(point, true),
            BoundaryCheck {
                position: expected,
                outside,
                outside_xz: outside
            }
        );
    }
    assert_ne!(projected_high, high, "upper projection must be asymmetric");
    assert_ne!(Fixed::from_f64(0.1).to_bits(), NATIVE_EPSILON_BITS);
}

#[test]
fn vertical_only_and_corners_report_axes_separately_and_inspection_preserves_input() {
    let bounds = reference();
    let unit = Fixed::ONE.to_bits();
    let xz_mid = 500 * unit;
    for (y, target) in [(-100 * unit - 1, -100 * unit), (100 * unit + 1, 100 * unit)] {
        let point = v(xz_mid, y, xz_mid);
        assert_eq!(
            bounds.check(point, false),
            BoundaryCheck {
                position: point,
                outside: true,
                outside_xz: false
            }
        );
        assert_eq!(
            bounds.check(point, true),
            BoundaryCheck {
                position: v(xz_mid, target, xz_mid),
                outside: true,
                outside_xz: false
            }
        );
    }
    for point in [
        v(-1, -100 * unit - 1, -1),
        v(1000 * unit + 1, 100 * unit + 1, 1000 * unit + 1),
    ] {
        let inspected = bounds.check(point, false);
        assert_eq!(
            inspected,
            BoundaryCheck {
                position: point,
                outside: true,
                outside_xz: true
            }
        );
        let enforced = bounds.check(point, true);
        assert!(enforced.outside && enforced.outside_xz);
        let inside = bounds.check(enforced.position, true);
        assert_eq!(
            inside,
            BoundaryCheck {
                position: enforced.position,
                outside: false,
                outside_xz: false
            }
        );
    }
}

#[test]
fn zero_padding_and_flat_y_are_valid_with_strict_vertical_neighbours() {
    let unit = Fixed::ONE.to_bits();
    let bounds = BoundaryVolume::new(
        v(0, 7 * unit, 0),
        v(2 * unit, 7 * unit, 2 * unit),
        Fixed::ZERO,
        Fixed::ZERO,
    )
    .unwrap();
    let on_plane = v(2 * unit, 7 * unit, 2 * unit);
    assert_eq!(
        bounds.check(on_plane, true),
        BoundaryCheck {
            position: on_plane,
            outside: false,
            outside_xz: false
        }
    );
    for point in [v(0, 7 * unit - 1, 0), v(0, 7 * unit + 1, 0)] {
        assert_eq!(
            bounds.check(point, true),
            BoundaryCheck {
                position: v(0, 7 * unit, 0),
                outside: true,
                outside_xz: false
            }
        );
    }
    let above = v(2 * unit + 1, 7 * unit, 2 * unit + 1);
    assert_eq!(
        bounds.check(above, true),
        BoundaryCheck {
            position: on_plane,
            outside: true,
            outside_xz: true
        }
    );
}

#[test]
fn constructor_rejects_each_invalid_volume_without_wrapping() {
    let zero = v(0, 0, 0);
    let ten = v(10, 10, 10);
    let plus_one = Fixed::from_bits(1);
    let minus_one = Fixed::from_bits(-1);
    assert_eq!(
        BoundaryVolume::new(zero, ten, minus_one, Fixed::ZERO),
        Err(BoundaryError::NegativeInset)
    );
    assert_eq!(
        BoundaryVolume::new(zero, ten, Fixed::ZERO, minus_one),
        Err(BoundaryError::NegativeUpperEpsilon)
    );
    for (min, max) in [(v(1, 0, 0), zero), (v(0, 1, 0), zero), (v(0, 0, 1), zero)] {
        assert_eq!(
            BoundaryVolume::new(min, max, Fixed::ZERO, Fixed::ZERO),
            Err(BoundaryError::ReversedBounds)
        );
    }
    assert_eq!(
        BoundaryVolume::new(zero, v(1, 0, 1), plus_one, Fixed::ZERO),
        Err(BoundaryError::InsufficientSpan)
    );
    assert_eq!(
        BoundaryVolume::new(zero, v(2, 0, 2), plus_one, plus_one),
        Err(BoundaryError::InsufficientSpan)
    );
    // Equality between lower and upper projection is the smallest valid span.
    let minimal =
        BoundaryVolume::new(zero, v(7, 0, 7), Fixed::from_bits(2), Fixed::from_bits(3)).unwrap();
    assert_eq!(minimal.check(v(5, 0, 5), true).position, v(5, 0, 5));
    assert_eq!(minimal.check(v(6, 0, 6), true).position, v(2, 0, 2));
    let largest = Fixed::from_bits(i64::MAX);
    let smallest = Fixed::from_bits(i64::MIN);
    assert_eq!(
        BoundaryVolume::new(
            v(largest.to_bits(), 0, 0),
            v(largest.to_bits(), 0, 0),
            plus_one,
            Fixed::ZERO
        ),
        Err(BoundaryError::DerivedLimitOverflow)
    );
    assert_eq!(
        BoundaryVolume::new(
            v(smallest.to_bits(), 0, smallest.to_bits()),
            v(largest.to_bits(), 0, smallest.to_bits()),
            plus_one,
            Fixed::ZERO
        ),
        Err(BoundaryError::DerivedLimitOverflow)
    );
    assert_eq!(
        BoundaryVolume::new(v(i64::MIN, 0, 0), v(i64::MIN, 0, 10), Fixed::ZERO, plus_one),
        Err(BoundaryError::DerivedLimitOverflow),
        "upper epsilon subtraction must be checked independently of inset"
    );
}

#[test]
fn valid_extreme_coordinates_and_deterministic_grid_enforce_to_fixed_point() {
    let extremes = BoundaryVolume::new(
        v(i64::MIN, i64::MIN, i64::MIN),
        v(i64::MAX, i64::MAX, i64::MAX),
        Fixed::ZERO,
        Fixed::ZERO,
    )
    .unwrap();
    for point in [
        v(i64::MIN, i64::MIN, i64::MIN),
        v(i64::MAX, i64::MAX, i64::MAX),
    ] {
        assert_eq!(
            extremes.check(point, true),
            BoundaryCheck {
                position: point,
                outside: false,
                outside_xz: false
            }
        );
    }
    let bounds = reference();
    let unit = Fixed::ONE.to_bits();
    for x in [-1, 0, 99, 100, 500, 900, 901, 1000, 1001] {
        for y in [-101, -100, 0, 100, 101] {
            for z in [-1, 0, 99, 100, 500, 900, 901, 1000, 1001] {
                let point = v(x * unit, y * unit, z * unit);
                let first = bounds.check(point, true);
                let second = bounds.check(first.position, true);
                assert_eq!(
                    second,
                    BoundaryCheck {
                        position: first.position,
                        outside: false,
                        outside_xz: false
                    },
                    "point {point:?}"
                );
                assert_eq!(
                    bounds.check(first.position, false),
                    second,
                    "projected point must also inspect inside: {point:?}"
                );
                assert_eq!(
                    bounds.check(point, false).position,
                    point,
                    "inspect changed {point:?}"
                );
            }
        }
    }
}
