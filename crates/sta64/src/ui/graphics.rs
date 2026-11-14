//! Applies explicit graphics choices after the renderer's startup resources exist.
//! Automatic restores the value each resource had before a player override.

use bevy::prelude::*;

use crate::frontend::screens::{SettingKey, Settings};
use crate::render::post::PostFx;
use crate::render::quality::RenderQuality;

pub struct GraphicsPlugin;

/// A player explicitly chose a graphics toggle, including its already-saved value.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct GraphicsEdited(pub SettingKey);

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<GraphicsEdited>().add_systems(PostUpdate, apply);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Choices {
    msaa: Option<u8>,
    detail: Option<u8>,
    shadows: Option<bool>,
    bloom: Option<bool>,
}

impl From<&Settings> for Choices {
    fn from(s: &Settings) -> Self {
        Self { msaa: s.msaa, detail: s.graphics_detail, shadows: s.shadows, bloom: s.bloom }
    }
}

#[derive(Default)]
struct Baseline {
    previous: Option<Choices>,
    quality: Option<RenderQuality>,
    bloom: Option<bool>,
}

fn apply(
    mut edits: MessageReader<GraphicsEdited>,
    settings: Option<Res<Settings>>,
    mut quality: Option<ResMut<RenderQuality>>,
    mut post: Option<ResMut<PostFx>>,
    mut baseline: Local<Baseline>,
) {
    let mut force_shadows = false;
    let mut force_bloom = false;
    for edit in edits.read() {
        match edit.0 {
            SettingKey::Shadows => force_shadows = true,
            SettingKey::Bloom => force_bloom = true,
            _ => {}
        }
    }
    let Some(settings) = settings else { return };
    let choices = Choices::from(settings.as_ref());
    let previous = baseline.previous;

    if let Some(quality) = quality.as_mut() {
        let fresh = quality.is_added() || baseline.quality.is_none();
        if fresh { baseline.quality = Some(**quality); }
        let original = baseline.quality.expect("quality baseline captured");
        if (fresh || previous.map(|p| p.msaa) != Some(choices.msaa))
            && let Some(msaa) = choices.msaa.or_else(|| previous.and_then(|p| p.msaa).map(|_| original.msaa))
            && quality.msaa != msaa
        {
            quality.msaa = msaa;
        }
        if (fresh || force_shadows || previous.map(|p| p.shadows) != Some(choices.shadows))
            && let Some(shadows) = choices.shadows.or_else(|| previous.and_then(|p| p.shadows).map(|_| original.shadows))
            && quality.shadows != shadows
        {
            quality.shadows = shadows;
        }
        if fresh || previous.map(|p| p.detail) != Some(choices.detail) {
            let detail = match choices.detail {
                Some(0) => Some((12, 4)),
                Some(1) => Some((0, 0)),
                Some(2) => Some((32, 1)),
                Some(_) => None,
                None if previous.is_some_and(|p| p.detail.is_some()) => Some((original.nebula_steps, original.nebula_scale)),
                None => None,
            };
            if let Some((steps, scale)) = detail {
                if quality.nebula_steps != steps { quality.nebula_steps = steps; }
                if quality.nebula_scale != scale { quality.nebula_scale = scale; }
            }
        }
    }

    if let Some(post) = post.as_mut() {
        let fresh = post.is_added() || baseline.bloom.is_none();
        if fresh { baseline.bloom = Some(post.bloom); }
        if (fresh || force_bloom || previous.map(|p| p.bloom) != Some(choices.bloom))
            && let Some(bloom) = choices.bloom.or_else(|| previous.and_then(|p| p.bloom).map(|_| baseline.bloom.expect("bloom baseline captured")))
            && post.bloom != bloom
        {
            post.bloom = bloom;
        }
    }
    baseline.previous = Some(choices);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::post::Aa;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<Settings>().add_plugins(GraphicsPlugin);
        app
    }

    #[test]
    fn untouched_and_automatic_restore_seeded_values_without_other_changes() {
        let mut app = app();
        let seed = RenderQuality { msaa: 2, shadows: false, depth_prepass: false, nebula_steps: 20, nebula_scale: 2 };
        let fx = PostFx { bloom: true, bloom_intensity: 0.42, aa: Aa::Fxaa, vignette: true, ..default() };
        app.insert_resource(seed).insert_resource(fx.clone());
        app.update();
        assert_eq!(*app.world().resource::<RenderQuality>(), seed);
        assert_eq!(*app.world().resource::<PostFx>(), fx);

        {
            let mut s = app.world_mut().resource_mut::<Settings>();
            s.msaa = Some(4);
            s.graphics_detail = Some(0);
            s.shadows = Some(true);
            s.bloom = Some(false);
        }
        app.update();
        let q = app.world().resource::<RenderQuality>();
        assert_eq!((q.msaa, q.shadows, q.nebula_steps, q.nebula_scale, q.depth_prepass), (4, true, 12, 4, false));
        let changed_fx = app.world().resource::<PostFx>();
        assert!(!changed_fx.bloom);
        assert_eq!(changed_fx.aa, Aa::Fxaa);
        assert_eq!(changed_fx.bloom_intensity, 0.42);
        assert!(changed_fx.vignette);

        {
            let mut s = app.world_mut().resource_mut::<Settings>();
            s.msaa = None;
            s.graphics_detail = None;
            s.shadows = None;
            s.bloom = None;
        }
        app.update();
        assert_eq!(*app.world().resource::<RenderQuality>(), seed);
        assert_eq!(*app.world().resource::<PostFx>(), fx);
    }

    #[test]
    fn resources_arriving_after_settings_capture_their_own_baselines() {
        let mut app = app();
        app.world_mut().resource_mut::<Settings>().graphics_detail = Some(2);
        app.world_mut().resource_mut::<Settings>().bloom = Some(false);
        app.update();
        let seed = RenderQuality { msaa: 1, shadows: false, depth_prepass: true, nebula_steps: 8, nebula_scale: 4 };
        app.insert_resource(seed).insert_resource(PostFx { bloom: true, ..default() });
        app.update();
        let q = app.world().resource::<RenderQuality>();
        assert_eq!((q.nebula_steps, q.nebula_scale, q.msaa, q.shadows), (32, 1, 1, false));
        assert!(!app.world().resource::<PostFx>().bloom);
        app.world_mut().resource_mut::<Settings>().graphics_detail = None;
        app.world_mut().resource_mut::<Settings>().bloom = None;
        app.update();
        assert_eq!(*app.world().resource::<RenderQuality>(), seed);
        assert!(app.world().resource::<PostFx>().bloom);
    }

    #[test]
    fn changing_msaa_does_not_reassert_bloom_after_runtime_preset_change() {
        let mut app = app();
        let seed = RenderQuality { msaa: 2, shadows: false, depth_prepass: false, nebula_steps: 18, nebula_scale: 2 };
        app.insert_resource(seed).insert_resource(PostFx { bloom: false, bloom_intensity: 0.37, ..default() });
        app.world_mut().resource_mut::<Settings>().bloom = Some(true);
        app.update();
        assert!(app.world().resource::<PostFx>().bloom);

        // F8 replaces the active post FX preset without changing the player's saved choice.
        *app.world_mut().resource_mut::<PostFx>() = PostFx { bloom: false, aa: Aa::Smaa, bloom_intensity: 0.37, ..default() };
        app.world_mut().resource_mut::<Settings>().msaa = Some(4);
        app.update();
        assert_eq!(*app.world().resource::<RenderQuality>(), RenderQuality { msaa: 4, ..seed });
        let fx = app.world().resource::<PostFx>();
        assert!(!fx.bloom);
        assert_eq!(fx.aa, Aa::Smaa);
        assert_eq!(fx.bloom_intensity, 0.37);
    }

    #[test]
    fn explicit_bloom_edit_reapplies_an_unchanged_saved_choice() {
        let mut app = app();
        app.insert_resource(RenderQuality { shadows: false, ..default() });
        app.insert_resource(PostFx { bloom: false, aa: Aa::Fxaa, bloom_intensity: 0.31, ..default() });
        app.world_mut().resource_mut::<Settings>().bloom = Some(true);
        app.update();
        assert!(app.world().resource::<PostFx>().bloom);

        // F8 changes the live preset; the saved preference still says true.
        *app.world_mut().resource_mut::<PostFx>() = PostFx { bloom: false, aa: Aa::Smaa, bloom_intensity: 0.31, ..default() };
        app.update();
        assert!(!app.world().resource::<PostFx>().bloom);
        app.world_mut().write_message(GraphicsEdited(SettingKey::Bloom));
        app.update();
        let fx = app.world().resource::<PostFx>();
        assert!(fx.bloom);
        assert_eq!(fx.aa, Aa::Smaa);
        assert_eq!(fx.bloom_intensity, 0.31);
        assert!(!app.world().resource::<RenderQuality>().shadows);
    }
}
