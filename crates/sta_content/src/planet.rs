//! Immutable planet rule families resolved by the profile-aware ODF loader.
//!
//! Role metadata does not itself grant mining, ownership, production or
//! colonization capabilities. Producer crew gain is a separate resolved input
//! for a derived capacity, not a crew stock or population count.

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
