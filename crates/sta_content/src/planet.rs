//! Immutable planet rule families resolved by the profile-aware ODF loader.
//!
//! Role metadata does not itself grant mining, ownership, production or
//! colonization capabilities. Producer crew gain is a separate resolved input
//! for a derived capacity, not a crew stock or population count.

use sta_core::Fixed;
use sta_odf::Odf;

use crate::{Profile, Warning};

/// A planet's rule family, independent of actor kind and occupied-colony state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanetRole {
    /// A background object, rather than a producer-backed colony planet.
    Background,
    /// The native producer-backed planet family. This does not imply that a
    /// colony exists, colonization is permitted, or production is active.
    Producer,
}

const ROLE_KEY: &str = "STA64_planetRole";

fn default_role(profile: Profile, class: &str) -> Option<PlanetRole> {
    match (profile, class) {
        (Profile::Sta1Classic | Profile::Sta2Classic | Profile::Fo4, "planet_obj") => Some(PlanetRole::Background),
        (Profile::Sta2Classic | Profile::Fo4, "planet") => Some(PlanetRole::Producer),
        _ => None,
    }
}

/// Resolve already layered ODF data once; programmatic definitions stay explicit.
pub(crate) fn resolve(file: &str, odf: &Odf, profile: Profile, warnings: &mut Vec<Warning>) -> Option<PlanetRole> {
    let class = odf.class_label().unwrap_or("").trim().to_ascii_lowercase();
    let fallback = default_role(profile, &class);
    let Some(raw) = odf.raw(ROLE_KEY) else { return fallback };
    if !matches!(class.as_str(), "planet_obj" | "planet") {
        warnings.push(Warning::new(
            file,
            format!("{ROLE_KEY} is unsupported for class {class:?}; ignored"),
        ));
        return None;
    }
    match odf.str(ROLE_KEY).unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "none" => None,
        "background" => Some(PlanetRole::Background),
        "producer" => Some(PlanetRole::Producer),
        _ => {
            warnings.push(Warning::new(
                file,
                format!("{ROLE_KEY} = {raw} is not none, background or producer; using profile/class default"),
            ));
            fallback
        }
    }
}

/// Resolve the effective producer contribution after the role is known.
/// Other roles do not read or diagnose an unused `maxCrewGain` key.
pub(crate) fn resolve_crew_gain(
    file: &str,
    odf: &Odf,
    profile: Profile,
    role: Option<PlanetRole>,
    warnings: &mut Vec<Warning>,
) -> i32 {
    if role != Some(PlanetRole::Producer) {
        return 0;
    }
    let fallback = if matches!(profile, Profile::Sta2Classic | Profile::Fo4) { 10_000 } else { 0 };
    let Some(raw) = odf.str("maxCrewGain") else { return fallback };
    match raw.trim().parse::<i32>() {
        Ok(value) => value,
        Err(_) => {
            warnings.push(Warning::new(
                file,
                format!("maxCrewGain = {raw} is not a signed 32-bit decimal integer; using profile default {fallback}"),
            ));
            fallback
        }
    }
}

/// Typed `crewModifier` of a Background planet. An authored value (including
/// zero) always wins. Only the Armada I profile has a measured absent-key
/// value, 1.2 (read from the executable, static); for other profiles an
/// absent key stays `None` (not measured), so a consumer cannot mistake it
/// for an authored multiplier. Other roles neither read nor diagnose the key.
pub(crate) fn resolve_crew_modifier(
    file: &str,
    odf: &Odf,
    profile: Profile,
    role: Option<PlanetRole>,
    warnings: &mut Vec<Warning>,
) -> Option<Fixed> {
    if role != Some(PlanetRole::Background) {
        return None;
    }
    let fallback = (profile == Profile::Sta1Classic).then(|| Fixed::from_f64(1.2));
    read_fixed(file, odf, "crewModifier", warnings).or(fallback)
}

/// Typed `civilianPopulationModifier` of a Producer planet; absent is 1 on
/// the Armada II family and zero elsewhere (an explicit Native or Armada I
/// opt-in has no measured native value). Authored zero stays zero.
pub(crate) fn resolve_civilian_population_modifier(
    file: &str,
    odf: &Odf,
    profile: Profile,
    role: Option<PlanetRole>,
    warnings: &mut Vec<Warning>,
) -> Fixed {
    if role != Some(PlanetRole::Producer) {
        return Fixed::ZERO;
    }
    let fallback = if is_a2_family(profile) { Fixed::ONE } else { Fixed::ZERO };
    read_fixed(file, odf, "civilianPopulationModifier", warnings).unwrap_or(fallback)
}

/// Typed `isColonizable` of a Producer planet: absent is true on the Armada
/// II family and false elsewhere. An explicit or inherited 0 stays false.
/// Strict flag spelling (`0`/`1`/`true`/`false`/`yes`/`no`); anything else
/// warns and uses the profile fallback.
pub(crate) fn resolve_colonizable(
    file: &str,
    odf: &Odf,
    profile: Profile,
    role: Option<PlanetRole>,
    warnings: &mut Vec<Warning>,
) -> bool {
    if role != Some(PlanetRole::Producer) {
        return false;
    }
    let fallback = is_a2_family(profile);
    let Some(raw) = odf.raw("isColonizable") else { return fallback };
    odf.bool("isColonizable").unwrap_or_else(|| {
        warnings.push(Warning::new(
            file,
            format!("isColonizable = {raw} is not a flag; using profile default {fallback}"),
        ));
        fallback
    })
}

fn is_a2_family(profile: Profile) -> bool {
    matches!(profile, Profile::Sta2Classic | Profile::Fo4)
}

/// A finite decimal number (stock files use a trailing `f`), clamped so the
/// fixed-point value cannot overflow; `None` with a warning when malformed.
fn read_fixed(file: &str, odf: &Odf, key: &str, warnings: &mut Vec<Warning>) -> Option<Fixed> {
    let raw = odf.raw(key)?;
    match odf.f32(key) {
        // Through the shortest decimal text so `1.2` is the same fixed value
        // as the literal fallback, not the widened f32 neighbour.
        Some(v) if v.is_finite() => {
            let exact = v.to_string().parse::<f64>().unwrap_or(v as f64);
            Some(Fixed::from_f64(exact.clamp(-1.0e9, 1.0e9)))
        }
        _ => {
            warnings.push(Warning::new(file, format!("{key} = {raw} is not a finite number; using the default")));
            None
        }
    }
}
