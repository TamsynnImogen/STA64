//! Quality settings for what trades look for speed (RND-31). Defaults are the look the
//! renderer had before the setting existed; the performance run (`bench/perf.rs`)
//! switches them to measure what each costs.

use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct RenderQuality {
    /// The key-light shadow pass (`shadows.rs`) may run (it still runs only when something
    /// needs it).
    pub shadows: bool,
    /// Multisampling of the map's cameras: 1 (off), 2 or 4 (Bevy's default).
    pub msaa: u8,
    /// The map camera's depth prepass (every opaque mesh drawn once more; the nebulae's march
    /// reads it).
    pub depth_prepass: bool,
    /// The nebula march's steps along a ray: 0 for [`default_nebula_steps`] of the resolution.
    pub nebula_steps: u32,
    /// The nebula march's resolution: 0 for [`auto_nebula_scale`] of the view's size, 1 (the
    /// view's own, one ray per pixel), 2 (half, per axis) or 4 (a quarter), drawn into a smaller target and laid over the view with a
    /// depth-aware upsample (`nebula_lowres.rs`).
    pub nebula_scale: u32,
}

/// The march's steps when the setting does not say (RND-34, measured): the look is tuned
/// for 16 at full size and at half size; a quarter-size march spends 12 (the gas's grain
/// is already a few pixels wide there; 8 doubles the difference to the full-size picture).
pub fn default_nebula_steps(divisor: u32) -> u32 {
    if divisor >= 4 {
        12
    } else {
        16
    }
}

/// The march's resolution divisor when the setting does not say (RND-34, measured): a
/// quarter from 1.5 megapixels up (a 1920x1080 view and wider), so the gas costs a few
/// milliseconds of GPU at those sizes instead of tens; a half below that.
pub fn auto_nebula_scale(width: u32, height: u32) -> u32 {
    if u64::from(width) * u64::from(height) >= 1_500_000 {
        4
    } else {
        2
    }
}

impl Default for RenderQuality {
    fn default() -> Self {
        RenderQuality { shadows: true, msaa: 4, depth_prepass: true, nebula_steps: 0, nebula_scale: 0 }
    }
}

impl RenderQuality {
    /// The defaults, with what the environment says (until the options screen has a graphics
    /// section): `STA64_SHADOWS=0`, `STA64_MSAA=1|2|4`, `STA64_NEBULA_SCALE=1|2|4` (unset: by the view's size), `STA64_NEBULA_STEPS=4..32`.
    pub fn from_env() -> RenderQuality {
        let mut q = RenderQuality::default();
        let var = |n: &str| std::env::var(n).ok();
        if var("STA64_SHADOWS").is_some_and(|v| v.trim() == "0") {
            q.shadows = false;
        }
        if let Some(m) = var("STA64_MSAA").and_then(|v| v.trim().parse::<u8>().ok()).filter(|m| matches!(m, 1 | 2 | 4)) {
            q.msaa = m;
        }
        if let Some(s) = var("STA64_NEBULA_SCALE").and_then(|v| v.trim().parse::<u32>().ok()).filter(|s| matches!(s, 1 | 2 | 4)) {
            q.nebula_scale = s;
        }
        if let Some(n) = var("STA64_NEBULA_STEPS").and_then(|v| v.trim().parse::<u32>().ok()).filter(|n| (4..=32).contains(n)) {
            q.nebula_steps = n;
        }
        q
    }
}

/// Measurement switches (the performance run's variants), not settings: a material the
/// camera moved is not rewritten, so what the writes cost shows by difference.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct PerfSwitches {
    pub freeze_rock_materials: bool,
    pub freeze_hull_materials: bool,
}

pub struct QualityPlugin;

impl Plugin for QualityPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RenderQuality::from_env()).init_resource::<PerfSwitches>().add_systems(Update, apply);
    }
}

/// Write the settings that live on cameras: to a camera made after the setting was chosen
/// (the match's cameras come with the match), and to every map camera when it changes.
/// The defaults touch nothing. Not the shadow camera, and not a camera post FX took the
/// multisampling of (FXAA/SMAA).
#[allow(clippy::type_complexity)]
fn apply(
    mut commands: Commands,
    quality: Res<RenderQuality>,
    mut last: Local<Option<RenderQuality>>,
    cameras: Query<
        (Entity, Has<DepthPrepass>, Has<crate::map::BackgroundCamera>),
        (With<Camera3d>, Without<crate::render::shadows::ShadowCamera>, Without<crate::render::post::PostFxMsaa>),
    >,
    added: Query<(), Added<Camera3d>>,
) {
    let prev = (*last).unwrap_or_default();
    *last = Some(*quality);
    let default = RenderQuality::default();
    let msaa = match quality.msaa {
        1 => Msaa::Off,
        2 => Msaa::Sample2,
        _ => Msaa::Sample4,
    };
    for (e, has, background) in &cameras {
        let fresh = added.contains(e);
        // A new camera gets what differs from the defaults it was made with; others what changed.
        let from = if fresh { default } else { prev };
        if from.msaa != quality.msaa {
            commands.entity(e).insert(msaa);
        }
        if !background && from.depth_prepass != quality.depth_prepass {
            if quality.depth_prepass && !has {
                commands.entity(e).insert(DepthPrepass);
            } else if !quality.depth_prepass && has {
                commands.entity(e).remove::<DepthPrepass>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nebula_march_defaults_follow_the_view_size() {
        assert_eq!(auto_nebula_scale(1280, 720), 2);
        assert_eq!(auto_nebula_scale(1920, 1080), 4);
        assert_eq!(auto_nebula_scale(3440, 1440), 4);
        assert_eq!((default_nebula_steps(1), default_nebula_steps(2), default_nebula_steps(4)), (16, 16, 12));
    }

    #[test]
    fn the_defaults_ask_for_the_automatic_nebula_settings() {
        assert_eq!(RenderQuality::default(), RenderQuality { shadows: true, msaa: 4, depth_prepass: true, nebula_steps: 0, nebula_scale: 0 });
    }
}
