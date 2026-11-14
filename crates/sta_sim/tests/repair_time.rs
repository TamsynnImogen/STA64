//! Synthetic coverage for the pure A2 modified repair-time policy.

use sta_core::Fixed;
use sta_sim::repair::{RepairTimeFactors, a2_modified_system_time};

fn factors() -> RepairTimeFactors {
    RepairTimeFactors {
        strength: Fixed::from_int(3),
        yellow: Fixed::from_int(2),
        red: Fixed::from_int(5),
        crew_yellow_status: Fixed::from_f64(0.5),
        crew_red_status: Fixed::from_f64(0.25),
    }
}

fn expected(numerator: i32, denominator: i32) -> Fixed {
    Fixed::from_int(numerator) / Fixed::from_int(denominator)
}

#[test]
fn authored_factors_cover_statuses_crew_resolution_and_hyper_repair() {
    let cases = [
        ("normal", Fixed::ONE, None, expected(15, 2)),
        ("yellow", Fixed::from_f64(0.5), None, Fixed::from_int(15)),
        ("red", Fixed::from_f64(0.25), None, Fixed::from_f64(37.5)),
        ("zero crew", Fixed::ZERO, None, Fixed::from_f64(37.5)),
        ("above one capped", Fixed::from_int(2), None, expected(15, 2)),
        (
            "hyper repair",
            Fixed::ONE,
            Some(expected(3, 2)),
            Fixed::from_int(5),
        ),
    ];

    for (label, crew, hyper, want) in cases {
        assert_eq!(
            a2_modified_system_time(expected(5, 2), crew, factors(), hyper),
            Some(want),
            "{label}"
        );
    }

    assert_eq!(
        a2_modified_system_time(Fixed::ZERO, Fixed::ONE, factors(), None),
        Some(Fixed::ZERO),
        "zero base"
    );
}

#[test]
fn status_thresholds_are_inclusive_at_adjacent_raw_values() {
    let half = Fixed::from_f64(0.5).to_bits();
    let quarter = Fixed::from_f64(0.25).to_bits();
    let cases = [
        (half + 1, expected(15, 2)),
        (half, Fixed::from_int(15)),
        (half - 1, Fixed::from_int(15)),
        (quarter + 1, Fixed::from_int(15)),
        (quarter, Fixed::from_f64(37.5)),
        (quarter - 1, Fixed::from_f64(37.5)),
    ];

    for (crew_bits, want) in cases {
        assert_eq!(
            a2_modified_system_time(
                expected(5, 2),
                Fixed::from_bits(crew_bits),
                factors(),
                None
            ),
            Some(want),
            "crew raw bits={crew_bits}"
        );
    }
}

#[test]
fn red_overrides_yellow_for_overlapping_or_reversed_thresholds() {
    let mut input = factors();
    input.crew_yellow_status = Fixed::from_f64(0.25);
    input.crew_red_status = Fixed::from_f64(0.75);
    assert_eq!(
        a2_modified_system_time(expected(5, 2), Fixed::from_f64(0.5), input, None),
        Some(Fixed::from_f64(37.5))
    );

    input.crew_yellow_status = Fixed::from_f64(0.75);
    input.crew_red_status = Fixed::from_f64(0.25);
    assert_eq!(
        a2_modified_system_time(expected(5, 2), Fixed::from_f64(0.5), input, None),
        Some(Fixed::from_int(15))
    );

    input.crew_yellow_status = Fixed::from_f64(0.5);
    input.crew_red_status = Fixed::from_f64(0.5);
    assert_eq!(
        a2_modified_system_time(expected(5, 2), Fixed::from_f64(0.5), input, None),
        Some(Fixed::from_f64(37.5))
    );
}

#[test]
fn crew_fraction_is_capped_before_status_selection() {
    let mut input = factors();
    input.crew_yellow_status = Fixed::ONE;
    assert_eq!(
        a2_modified_system_time(expected(5, 2), Fixed::from_int(2), input, None),
        Some(Fixed::from_int(15))
    );
}

#[test]
fn absent_hyper_repair_means_one_but_zero_divisor_is_rejected() {
    let base = expected(5, 2);
    assert_eq!(
        a2_modified_system_time(base, Fixed::ONE, factors(), None),
        Some(expected(15, 2))
    );
    assert_eq!(
        a2_modified_system_time(base, Fixed::ONE, factors(), Some(Fixed::ZERO)),
        None
    );
}

#[test]
fn signed_inputs_are_preserved_and_q32_rounding_is_explicit() {
    let mut input = factors();
    input.crew_yellow_status = Fixed::ONE;
    input.yellow = Fixed::from_bits(-1);
    input.strength = Fixed::from_f64(0.5);
    assert_eq!(
        a2_modified_system_time(Fixed::ONE, Fixed::ONE, input, None),
        Some(Fixed::from_bits(-1))
    );

    // Division truncates toward zero after the negative raw-bit product.
    input.yellow = Fixed::ONE;
    assert_eq!(
        a2_modified_system_time(
            Fixed::from_bits(-1),
            Fixed::ONE,
            input,
            Some(Fixed::from_int(2))
        ),
        Some(Fixed::ZERO)
    );
}

#[test]
fn overflowing_intermediate_is_rejected_before_later_division() {
    let mut input = factors();
    input.strength = Fixed::from_bits(i64::MAX);
    assert_eq!(
        a2_modified_system_time(
            Fixed::from_int(2),
            Fixed::ONE,
            input,
            Some(Fixed::from_int(2))
        ),
        None
    );
}

#[test]
fn first_multiply_overflow_is_rejected() {
    let mut input = factors();
    input.strength = Fixed::from_bits(i64::MAX);
    assert_eq!(
        a2_modified_system_time(
            Fixed::ONE,
            Fixed::from_f64(0.5),
            input,
            None
        ),
        None
    );
}

#[test]
fn tiny_divisor_rejects_an_overflowing_quotient() {
    let mut input = factors();
    input.strength = Fixed::ONE;
    assert_eq!(
        a2_modified_system_time(
            Fixed::ONE,
            Fixed::ONE,
            input,
            Some(Fixed::from_bits(1))
        ),
        None
    );
}

#[test]
fn fixed_minimum_and_maximum_survive_identity_multipliers() {
    let mut input = factors();
    input.strength = Fixed::ONE;
    for edge in [Fixed::from_bits(i64::MIN), Fixed::from_bits(i64::MAX)] {
        assert_eq!(
            a2_modified_system_time(edge, Fixed::ONE, input, None),
            Some(edge)
        );
    }
}
