//! Armada I's physics keys for the movement model (CON-52).
//!
//! Rule: under [`Profile::Sta1Classic`] only, a craft flies with the *play
//! physics* integrator keys (one shared file, [`Profile::play_physics_file`])
//! and its *own* physics file's order keys and speeds. The own file is the one
//! the unit ODF names with `physicsFile` (reached through the unit's
//! `#include` chain, so craft that share a base ODF share a physics file); the
//! play file is read from the same layered ODF set, so a mod overrides it by
//! shipping a file of that name, and a unit's own file by the usual layering.
//! Keys absent from a file take the executable's defaults (key names and
//! grade only: read from the executable, static). A play block that is not
//! usable (a non-positive `impulseSpeed`,
//! `frontBackInertia`, `turnRadius`, `velocWhenCoeffHalf` or
//! `braccelControlStiffness`) is dropped with a warning (a missing file quietly)
//! and the model stays
//! off: every Armada I craft keeps the old movement. Nothing here is read, or
//! hashed, under another profile.

use sta_core::Fixed;
use sta_odf::Odf;

use crate::odf::Reader;
use crate::Warning;

/// The play-physics file's keys the model reads (the file the executable names
/// `fastphys.odf`; every steered craft integrates with it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct A1Dynamics {
    /// `impulseSpeed` of the play physics: the scale of the lag, the turn rate
    /// and the speed curve.
    pub impulse_speed: Fixed,
    /// `frontBackInertia`: seconds times impulse; the lag is this divided by
    /// `impulse_speed`.
    pub front_back_inertia: Fixed,
    /// `maxReverseSpeed`: a fraction of impulse.
    pub max_reverse_speed: Fixed,
    /// `turnRadius`: world units at impulse speed and full servo.
    pub turn_radius: Fixed,
    /// `velocCoeffStill`: turn authority at rest, 0 to 1.
    pub veloc_coeff_still: Fixed,
    /// `velocWhenCoeffHalf`: speed at half authority, a fraction of impulse.
    pub veloc_when_coeff_half: Fixed,
    /// `braccelControlStiffness`: stiffness of the throttle controller, 1/s^2.
    pub braccel_control_stiffness: Fixed,
    /// `sy_yawAngleCoeff`: servo gain on the heading error (radians).
    pub yaw_angle_coeff: Fixed,
    /// `sy_yawVelocCoeff`: servo damping on the turn rate.
    pub yaw_veloc_coeff: Fixed,
    /// `rollRotationSpeed`: a fraction of impulse; sets the turn-rate lag.
    pub roll_rotation_speed: Fixed,
}

/// A craft's own Borg keys (`borgPhysics` is 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct A1Borg {
    /// `maxRotationSpeed`, radians per second.
    pub max_rotation_speed: Fixed,
    /// `rotationAcceleration`, 1/s.
    pub rotation_acceleration: Fixed,
    /// `rotationStiffness`, 1/s^2.
    pub rotation_stiffness: Fixed,
    /// `tooSlowToTurn`, units per second.
    pub too_slow_to_turn: Fixed,
}


/// The executable's defaults for an absent key (key names only; read from the
/// executable, static).
const DEFAULTS: [(&str, f64); 14] = [
    ("frontBackInertia", 2.0),
    ("maxReverseSpeed", 15.0),
    ("turnRadius", 25.0),
    ("velocCoeffStill", 0.1),
    ("velocWhenCoeffHalf", 7.5),
    ("braccelControlStiffness", 10.0),
    ("sy_yawAngleCoeff", 0.726),
    ("sy_yawVelocCoeff", -0.043),
    ("rollRotationSpeed", -1.0),
    ("maxRotationSpeed", 0.25),
    ("rotationAcceleration", 2.0),
    ("rotationStiffness", 1.0),
    ("tooSlowToTurn", 1.0),
    ("impulseSpeed", -1.0),
];

fn key(r: &mut Reader<'_>, name: &str) -> Fixed {
    let default = DEFAULTS.iter().find(|(k, _)| *k == name).map_or(0.0, |(_, v)| *v);
    Fixed::from_f64(r.num(name).unwrap_or(default))
}

/// The play-physics block of the ODF `odf` named `file`, or `None` (with a
/// warning) when it is not usable.
pub(crate) fn play_dynamics(odf: &Odf, file: &str, warnings: &mut Vec<Warning>) -> Option<A1Dynamics> {
    let mut r = Reader { odf, file, warnings };
    let d = A1Dynamics {
        impulse_speed: key(&mut r, "impulseSpeed"),
        front_back_inertia: key(&mut r, "frontBackInertia"),
        max_reverse_speed: key(&mut r, "maxReverseSpeed"),
        turn_radius: key(&mut r, "turnRadius"),
        veloc_coeff_still: key(&mut r, "velocCoeffStill"),
        veloc_when_coeff_half: key(&mut r, "velocWhenCoeffHalf"),
        braccel_control_stiffness: key(&mut r, "braccelControlStiffness"),
        yaw_angle_coeff: key(&mut r, "sy_yawAngleCoeff"),
        yaw_veloc_coeff: key(&mut r, "sy_yawVelocCoeff"),
        roll_rotation_speed: key(&mut r, "rollRotationSpeed"),
    };
    let must_be_positive = [
        ("impulseSpeed", d.impulse_speed),
        ("frontBackInertia", d.front_back_inertia),
        ("turnRadius", d.turn_radius),
        ("velocWhenCoeffHalf", d.veloc_when_coeff_half),
        ("braccelControlStiffness", d.braccel_control_stiffness),
    ];
    if let Some((name, _)) = must_be_positive.iter().find(|(_, v)| *v <= Fixed::ZERO) {
        warnings.push(Warning::new(file, format!("{name} is not positive; the Armada I movement model stays off")));
        return None;
    }
    Some(d)
}

/// The Borg keys of a craft's own physics ODF: `Some` when `borgPhysics` is
/// set to a non-zero number.
pub(crate) fn borg(phys: &Odf, file: &str, warnings: &mut Vec<Warning>) -> Option<A1Borg> {
    let mut r = Reader { odf: phys, file, warnings };
    if r.num("borgPhysics").unwrap_or(0.0) == 0.0 {
        return None;
    }
    Some(A1Borg {
        max_rotation_speed: key(&mut r, "maxRotationSpeed"),
        rotation_acceleration: key(&mut r, "rotationAcceleration"),
        rotation_stiffness: key(&mut r, "rotationStiffness"),
        too_slow_to_turn: key(&mut r, "tooSlowToTurn"),
    })
}

/// The digest text of a play block (field order fixed).
pub(crate) fn dynamics_text(d: &A1Dynamics) -> String {
    let f = crate::digest::fixed_text;
    format!(
        "[{},{},{},{},{},{},{},{},{},{}]",
        f(d.impulse_speed), f(d.front_back_inertia), f(d.max_reverse_speed), f(d.turn_radius), f(d.veloc_coeff_still),
        f(d.veloc_when_coeff_half), f(d.braccel_control_stiffness), f(d.yaw_angle_coeff), f(d.yaw_veloc_coeff), f(d.roll_rotation_speed)
    )
}

/// The digest text of a Borg block.
pub(crate) fn borg_text(b: &A1Borg) -> String {
    let f = crate::digest::fixed_text;
    format!("[{},{},{},{}]", f(b.max_rotation_speed), f(b.rotation_acceleration), f(b.rotation_stiffness), f(b.too_slow_to_turn))
}
