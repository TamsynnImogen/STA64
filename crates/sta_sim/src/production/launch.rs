//! Launch points (SIM-16): a finished unit appears at its builder's launch
//! hardpoint, faces out, and, with no rally order, drives out of the builder's
//! clear circle. The hardpoint comes from the builder's def
//! ([`LaunchPoint`](sta_content::LaunchPoint), read by `sta_content::launch`);
//! a builder without one keeps the old behaviour (centre, own heading).

use sta_content::{Fixed, UnitDef, Vec3};

/// Margin past the clear circle, units.
const MARGIN: i32 = 20;

/// Where a new unit appears, which way it faces, and the first point it
/// drives to so that it is clear of the builder.
pub(super) struct Launch {
    pub pos: Vec3,
    pub heading: (Fixed, Fixed),
    pub exit: Option<Vec3>,
}

/// Local `(x, z)` (+z ahead) turned to world by the builder's heading.
fn turn(h: (Fixed, Fixed), v: (Fixed, Fixed)) -> (Fixed, Fixed) {
    (v.0 * h.1 + v.1 * h.0, v.1 * h.1 - v.0 * h.0)
}

pub(super) fn place(builder: &UnitDef, pos: Vec3, heading: (Fixed, Fixed)) -> Launch {
    let Some(l) = &builder.launch else {
        return Launch {
            pos,
            heading,
            exit: None,
        };
    };
    let (ox, oz) = turn(heading, l.offset);
    let dir = turn(heading, l.dir);
    let at = Vec3 {
        x: pos.x + ox,
        y: pos.y,
        z: pos.z + oz,
    };
    // Distance t along `dir` from the hardpoint to the circle of radius
    // `clear + MARGIN`: |o + t d| = R, with |d| = 1.
    let r = l.clear + Fixed::from_int(MARGIN);
    let b = ox * dir.0 + oz * dir.1;
    let disc = r * r - (ox * ox + oz * oz) + b * b;
    let t = if disc > Fixed::ZERO {
        disc.sqrt() - b
    } else {
        Fixed::ZERO
    };
    let t = if t > Fixed::ZERO { t } else { Fixed::ZERO };
    let exit = Vec3 {
        x: at.x + dir.0 * t,
        y: at.y,
        z: at.z + dir.1 * t,
    };
    Launch {
        pos: at,
        heading: dir,
        exit: Some(exit),
    }
}

/// Whether `p` is outside the circle `l` leaves around a builder at `at`.
#[cfg(test)]
pub(super) fn is_clear(builder: &UnitDef, at: Vec3, p: Vec3) -> bool {
    let Some(l) = &builder.launch else {
        return true;
    };
    let (dx, dz) = (p.x - at.x, p.z - at.z);
    (dx * dx + dz * dz).sqrt() >= l.clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_content::LaunchPoint;

    fn n(v: i32) -> Fixed {
        Fixed::from_int(v)
    }

    fn yard() -> UnitDef {
        let mut d = UnitDef::new("y", "shipyard");
        d.launch = Some(LaunchPoint {
            offset: (n(10), n(0)),
            dir: (n(0), n(1)),
            clear: n(100),
            node: Some("hp99".into()),
        });
        d
    }

    #[test]
    fn no_launch_point_is_the_old_behaviour() {
        let d = UnitDef::new("y", "shipyard");
        let at = Vec3 {
            x: n(5),
            y: n(0),
            z: n(7),
        };
        let l = place(&d, at, (n(1), n(0)));
        assert_eq!((l.pos, l.heading, l.exit), (at, (n(1), n(0)), None));
    }

    #[test]
    fn offset_and_direction_follow_the_builder_heading() {
        let d = yard();
        let at = Vec3 {
            x: n(0),
            y: n(0),
            z: n(0),
        };
        let a = place(&d, at, (n(0), n(1)));
        assert_eq!((a.pos.x, a.pos.z, a.heading), (n(10), n(0), (n(0), n(1))));
        // Turned a quarter: local +x becomes world -z, local +z becomes +x.
        let b = place(&d, at, (n(1), n(0)));
        assert_eq!((b.pos.x, b.pos.z, b.heading), (n(0), n(-10), (n(1), n(0))));
    }

    #[test]
    fn the_exit_is_clear_of_the_builder() {
        let d = yard();
        let at = Vec3 {
            x: n(30),
            y: n(0),
            z: n(-40),
        };
        let l = place(&d, at, (n(0), n(1)));
        assert!(is_clear(&d, at, l.exit.unwrap()));
        assert!(!is_clear(&d, at, l.pos));
    }
}
