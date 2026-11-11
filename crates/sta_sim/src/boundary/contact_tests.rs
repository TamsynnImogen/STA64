use sta_core::{Fixed, Vec3};

use super::{contact, distance, legal_target, product};
use crate::boundary::BoundaryVolume;
use crate::spatial::Solid;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

fn volume() -> BoundaryVolume {
    BoundaryVolume::new(
        v(-500, -500),
        v(500, 500),
        Fixed::from_int(100),
        Fixed::from_bits(429496736),
    )
    .unwrap()
}

fn body() -> Fixed {
    Fixed::from_int(75)
}

#[test]
fn legal_target_checks_volume_clearance_and_exact_short_segment() {
    let bounds = BoundaryVolume::new(v(-10, -10), v(10, 10), Fixed::ZERO, Fixed::ZERO).unwrap();
    let raw = |x, z| Vec3 {
        x: Fixed::from_bits(x),
        y: Fixed::ZERO,
        z: Fixed::from_bits(z),
    };
    let tiny = Solid::Sphere {
        centre: raw(-1, 2),
        radius: Fixed::from_bits(2),
    };
    assert!(legal_target(
        &bounds,
        raw(0, 0),
        raw(1, 1),
        Fixed::ZERO,
        &[tiny]
    ));
    assert!(!legal_target(
        &bounds,
        raw(-3, 2),
        raw(-1, 2),
        Fixed::ZERO,
        &[tiny]
    ));
    assert!(!legal_target(&bounds, v(0, 0), v(11, 0), Fixed::ZERO, &[]));
    assert!(legal_target(&bounds, v(11, 0), v(10, 0), Fixed::ZERO, &[]));
    assert!(!legal_target(&bounds, v(0, 0), v(0, 0), -Fixed::ONE, &[]));

    let crossing = Solid::Sphere {
        centre: v(0, 0),
        radius: Fixed::from_int(2),
    };
    assert!(!legal_target(
        &bounds,
        v(-5, 0),
        v(5, 0),
        Fixed::ZERO,
        &[crossing]
    ));
    assert!(legal_target(
        &bounds,
        v(0, 0),
        v(5, 0),
        Fixed::ZERO,
        &[crossing]
    ));
}

#[test]
fn sphere_contacts_recover_near_every_comparison_border_without_clipping_into_blocker() {
    let bounds = volume();
    let radius = body();
    let margin = Fixed::from_int(12);
    for (centre, origin, desired) in [
        (v(350, 0), v(350, -300), v(467, 0)),
        (v(-350, 0), v(-350, -300), v(-467, 0)),
        (v(0, 350), v(-300, 350), v(0, 467)),
        (v(0, -350), v(-300, -350), v(0, -467)),
        (v(350, 350), v(0, 0), v(467, 467)),
    ] {
        let blocker = Solid::Sphere {
            centre,
            radius: Fixed::from_int(30),
        };
        let result = contact(
            &bounds,
            origin,
            desired,
            &blocker,
            radius,
            margin,
            &[blocker],
        )
        .unwrap();
        assert!(!bounds.check(result, false).outside);
        assert!(legal_target(&bounds, origin, result, radius, &[blocker]));
        assert_ne!(result, bounds.check(desired, true).position);
    }
}

#[test]
fn contact_checks_blocker_even_when_omitted_and_chooses_reachable_box_face() {
    let bounds = volume();
    let wall = Solid::Box {
        min: v(350, -40),
        max: v(390, 40),
    };
    let result = contact(
        &bounds,
        v(200, 0),
        v(467, 0),
        &wall,
        body(),
        Fixed::from_int(12),
        &[],
    );
    assert_eq!(result, Some(v(263, 0)));
    assert!(legal_target(
        &bounds,
        v(200, 0),
        result.unwrap(),
        body(),
        &[wall]
    ));

    let separating = Solid::Box {
        min: v(-50, -500),
        max: v(50, 500),
    };
    let opposite = contact(
        &bounds,
        v(-300, 0),
        v(137, 0),
        &separating,
        body(),
        Fixed::from_int(12),
        &[],
    );
    assert_eq!(opposite, Some(v(-137, 0)));
}

#[test]
fn contacts_respect_other_solids_and_circle_intersections_when_cardinals_are_outside() {
    let bounds = volume();
    let centre = Solid::Sphere {
        centre: v(350, 0),
        radius: Fixed::from_int(30),
    };
    let blockers = [
        centre,
        Solid::Sphere {
            centre: v(350, -200),
            radius: Fixed::from_int(30),
        },
        Solid::Sphere {
            centre: v(350, 200),
            radius: Fixed::from_int(30),
        },
    ];
    let selected = contact(
        &bounds,
        v(0, 0),
        v(467, 0),
        &centre,
        body(),
        Fixed::from_int(12),
        &blockers,
    );
    assert_eq!(selected, Some(v(233, 0)));
    assert!(legal_target(
        &bounds,
        v(0, 0),
        selected.unwrap(),
        body(),
        &blockers
    ));

    let large = Solid::Sphere {
        centre: v(0, 0),
        radius: Fixed::from_int(400),
    };
    let corner = contact(
        &bounds,
        v(-400, -400),
        v(487, 0),
        &large,
        body(),
        Fixed::from_int(12),
        &[large],
    )
    .unwrap();
    assert!(corner.x.abs() == Fixed::from_int(400) || corner.z.abs() == Fixed::from_int(400));
    assert!(legal_target(
        &bounds,
        v(-400, -400),
        corner,
        body(),
        &[large]
    ));

    let covered = Solid::Sphere {
        centre: v(0, 0),
        radius: Fixed::from_int(1000),
    };
    assert_eq!(
        contact(
            &bounds,
            v(0, 0),
            v(1087, 0),
            &covered,
            body(),
            Fixed::from_int(12),
            &[covered]
        ),
        None
    );
}

#[test]
fn selected_contact_uses_projected_desired_y_and_stable_candidate_order() {
    let bounds = BoundaryVolume::new(
        Vec3 {
            y: Fixed::from_int(-500),
            ..v(-500, -500)
        },
        Vec3 {
            y: Fixed::from_int(500),
            ..v(500, 500)
        },
        Fixed::from_int(100),
        Fixed::from_bits(429496736),
    )
    .unwrap();
    let sphere = Solid::Sphere {
        centre: v(0, 0),
        radius: Fixed::from_int(10),
    };
    let desired = Vec3 {
        y: Fixed::from_int(600),
        ..v(0, 0)
    };
    let chosen = contact(
        &bounds,
        v(0, 0),
        desired,
        &sphere,
        Fixed::ZERO,
        Fixed::ZERO,
        &[],
    )
    .unwrap();
    assert_eq!(
        chosen,
        Vec3 {
            y: Fixed::from_int(500),
            ..v(-10, 0)
        }
    );
    assert!(legal_target(
        &bounds,
        v(0, 0),
        chosen,
        Fixed::ZERO,
        &[sphere]
    ));
}

#[test]
fn invalid_or_unprovable_geometry_returns_none_without_panicking() {
    let bounds = volume();
    let sphere = Solid::Sphere {
        centre: v(0, 0),
        radius: Fixed::from_int(10),
    };
    assert_eq!(
        contact(
            &bounds,
            v(0, 0),
            v(20, 0),
            &sphere,
            -Fixed::ONE,
            Fixed::ZERO,
            &[]
        ),
        None
    );
    assert_eq!(
        contact(
            &bounds,
            v(0, 0),
            v(20, 0),
            &sphere,
            Fixed::ZERO,
            -Fixed::ONE,
            &[]
        ),
        None
    );
    assert_eq!(
        contact(
            &bounds,
            v(0, 0),
            v(20, 0),
            &sphere,
            Fixed::from_bits(i64::MAX),
            Fixed::ONE,
            &[]
        ),
        None
    );
    let reversed = Solid::Box {
        min: v(5, 5),
        max: v(-5, -5),
    };
    assert_eq!(
        contact(
            &bounds,
            v(0, 0),
            v(20, 0),
            &reversed,
            Fixed::ZERO,
            Fixed::ZERO,
            &[]
        ),
        None
    );
    assert!(!legal_target(
        &bounds,
        v(0, 0),
        v(20, 0),
        Fixed::ZERO,
        &[reversed]
    ));
}

#[test]
fn exact_products_and_distance_keys_cover_widest_raw_values() {
    assert_eq!(product(u128::MAX, u128::MAX), (u128::MAX - 1, 1));
    let min = Vec3 {
        x: Fixed::from_bits(i64::MIN),
        y: Fixed::ZERO,
        z: Fixed::from_bits(i64::MIN),
    };
    let max = Vec3 {
        x: Fixed::from_bits(i64::MAX),
        y: Fixed::ZERO,
        z: Fixed::from_bits(i64::MAX),
    };
    assert!(distance(min, max).0);
}
