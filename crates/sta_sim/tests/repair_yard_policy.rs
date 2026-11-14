//! Synthetic coverage for the pure A2 yard-selection policy.

use sta_core::{EntityId, Fixed};
use sta_sim::repair::{YardCandidate, select_a2_yard};

fn candidate(id: u32, distance: i64, queued: u32) -> YardCandidate {
    YardCandidate {
        id: EntityId(id),
        surface_distance: Fixed::from_bits(distance),
        queued,
    }
}

#[test]
fn far_free_yard_can_beat_close_busy_yard() {
    let speed = Fixed::from_int(10);
    let candidates = [
        candidate(1, Fixed::from_int(70).to_bits(), 4),
        candidate(2, Fixed::from_int(170).to_bits(), 0),
    ];
    assert_eq!(select_a2_yard(&candidates, speed), Some(EntityId(2)));
}

#[test]
fn reference_outcomes_across_speed_and_queue_values() {
    let queue_values = [0, 1, 4, 20];
    let speeds = [1, 10, 100];
    let expected_indices = [[0, 0, 0, 0], [0, 0, 1, 1], [0, 1, 1, 1]];

    for (speed_index, speed) in speeds.into_iter().enumerate() {
        for (queue_index, queue) in queue_values.into_iter().enumerate() {
            let candidates = [
                candidate(1, Fixed::from_int(70).to_bits(), queue),
                candidate(2, Fixed::from_int(170).to_bits(), 0),
            ];
            let expected = EntityId(expected_indices[speed_index][queue_index] as u32 + 1);
            assert_eq!(
                select_a2_yard(&candidates, Fixed::from_int(speed)),
                Some(expected),
                "speed={speed}, queued={queue}"
            );
        }
    }
}

#[test]
fn exact_tie_preserves_first_candidate_in_either_order() {
    let speed = Fixed::from_int(10);
    let forward = [
        candidate(9, Fixed::from_int(170).to_bits(), 0),
        candidate(2, Fixed::from_int(70).to_bits(), 2),
    ];
    let reverse = [forward[1], forward[0]];
    assert_eq!(select_a2_yard(&forward, speed), Some(EntityId(9)));
    assert_eq!(select_a2_yard(&reverse, speed), Some(EntityId(2)));
}

#[test]
fn negative_surface_distances_are_not_clamped() {
    let candidates = [candidate(7, -10, 0), candidate(3, -1, 0)];
    assert_eq!(select_a2_yard(&candidates, Fixed::ONE), Some(EntityId(7)));
}

#[test]
fn maximum_queue_length_is_compared_without_narrowing() {
    let candidates = [candidate(1, 0, u32::MAX), candidate(2, 1, 0)];
    assert_eq!(select_a2_yard(&candidates, Fixed::ONE), Some(EntityId(2)));
}

#[test]
fn smallest_positive_speed_is_supported() {
    let candidates = [candidate(1, 1, 1), candidate(2, 2, 0)];
    assert_eq!(
        select_a2_yard(&candidates, Fixed::from_bits(1)),
        Some(EntityId(2))
    );
}

#[test]
fn extreme_fixed_values_and_queue_do_not_overflow_score() {
    let candidates = [candidate(1, i64::MIN, u32::MAX), candidate(2, i64::MAX, 0)];
    assert_eq!(
        select_a2_yard(&candidates, Fixed::from_bits(i64::MAX)),
        Some(EntityId(2))
    );
}

#[test]
fn ids_are_not_resorted() {
    let candidates = [candidate(9, 100, 0), candidate(2, 100, 0)];
    assert_eq!(select_a2_yard(&candidates, Fixed::ONE), Some(EntityId(9)));
}

#[test]
fn empty_candidates_and_nonpositive_speeds_have_no_selection() {
    assert_eq!(select_a2_yard(&[], Fixed::ONE), None);
    let candidates = [candidate(1, 0, 0)];
    assert_eq!(select_a2_yard(&candidates, Fixed::ZERO), None);
    assert_eq!(select_a2_yard(&candidates, Fixed::from_bits(-1)), None);
}
