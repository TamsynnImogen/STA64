//! Object culling distance, fade band, clip planes and model LOD distances (RND-07).
//!
//! The values are the game's own, read from the mounted `ART_CFG.h` (the active
//! mod's, else the base game's, as the VFS resolves it); the numbers below are only
//! the defaults for a file that does not carry them:
//!
//! | value | `ART_CFG.h` key | default |
//! |---|---|---|
//! | culling distance | `cfgOBJECT_CULLING_DISTANCE` | 2800 |
//! | far clip plane | `FAR_CLIPPING_PLANE` | 20000 |
//! | near clip plane | `NEAR_CLIPPING_PLANE` | 20 |
//! | LOD distances | `DETAIL_2_LOD_0..3_DISTANCE` (middle detail level) | 1000, 500, 300, 100 |
//!
//! The fade is not a configuration value in the games: it is the engine's
//! `GameObjectInstance::ComputeFadeOut` (Refit `render3d/README.md`, "View
//! distance"): an object fades from [`FADE_FROM`] (0.9) of the culling distance to
//! all of it, by its distance from the camera with the vertical scaled by a quarter
//! ([`VERTICAL_SCALE`]). Stock also scales the distance by the object's radius for
//! some classes; that is not done here.
//!
//! What STA64 does with it: [`CullParams::fade`] is 1 inside the band, falls to 0 at
//! the culling distance and the strategic handoff ([`super::lod_switch`]) takes the
//! object from its model to its symbol there instead of letting it vanish, and
//! back only when the fade is full again (so the band is the hysteresis). No
//! per-material alpha fade of the model is applied: the model changes to the symbol
//! in one frame, never both. The far and near planes are recorded but not applied to
//! the camera (the map's camera owns them and its sky and nebulae reach past 20000).
//!
//! Render marks: `render.view` once the values are read (Refit's `planet` scene logs
//! `view: ART_CFG.h parsed: far plane now .., object culling now ..` for the same
//! two values), and `render.cull` whenever the counts of culled and faded objects
//! change.

use bevy::prelude::*;
use sta_assets::cfg_h::CfgH;
use sta_bench::marks::{Mark, Val};

use crate::bench::marks::RenderMarks;
use crate::clock::AnimClock;
use crate::viewer::Mounted;

/// `cfgOBJECT_CULLING_DISTANCE`'s value in every shipped file.
pub const DEFAULT_CULL: f32 = 2800.0;
/// An object starts to fade at this share of the culling distance.
pub const FADE_FROM: f32 = 0.9;
pub const DEFAULT_FAR: f32 = 20000.0;
pub const DEFAULT_NEAR: f32 = 20.0;
/// The engine counts the height of an object over the camera a quarter.
pub const VERTICAL_SCALE: f32 = 0.25;
/// `DETAIL_n_LOD_0..3_DISTANCE` of every shipped file.
pub const DEFAULT_LOD: [f32; 4] = [1000.0, 500.0, 300.0, 100.0];

/// The distances in force.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CullParams {
    /// Past this an object is not drawn.
    pub cull: f32,
    /// Share of `cull` where the fade starts.
    pub fade_from: f32,
    pub far: f32,
    pub near: f32,
    /// Camera distance inside which the next, finer LOD shows.
    pub lod: [f32; 4],
}

impl Default for CullParams {
    fn default() -> Self {
        CullParams { cull: DEFAULT_CULL, fade_from: FADE_FROM, far: DEFAULT_FAR, near: DEFAULT_NEAR, lod: DEFAULT_LOD }
    }
}

impl CullParams {
    /// Read `ART_CFG.h`; a key it lacks (or sets to nothing useful) keeps the default.
    pub fn from_cfg(cfg: Option<&CfgH>) -> CullParams {
        let d = CullParams::default();
        let Some(c) = cfg else { return d };
        let pos = |k: &str, v: f32| c.f32(k).filter(|x| *x > 0.0).unwrap_or(v);
        let mut lod = d.lod;
        for (i, l) in lod.iter_mut().enumerate() {
            *l = pos(&format!("DETAIL_2_LOD_{i}_DISTANCE"), *l);
        }
        CullParams { cull: pos("cfgOBJECT_CULLING_DISTANCE", d.cull), fade_from: d.fade_from, far: pos("FAR_CLIPPING_PLANE", d.far), near: pos("NEAR_CLIPPING_PLANE", d.near), lod }
    }

    /// The engine's distance of an object from the camera, for culling.
    pub fn distance(eye: Vec3, pos: Vec3) -> f32 {
        let d = pos - eye;
        Vec3::new(d.x, d.y * VERTICAL_SCALE, d.z).length()
    }

    /// 1 up to the start of the band, falling linearly to 0 at the culling distance.
    pub fn fade(&self, distance: f32) -> f32 {
        let start = self.cull * self.fade_from;
        if distance <= start {
            1.0
        } else if distance >= self.cull {
            0.0
        } else {
            (self.cull - distance) / (self.cull - start)
        }
    }

    /// The LOD index the engine picks at a distance: how many of the four distances
    /// the camera is inside (0, the coarsest, past the first). A model with fewer
    /// levels shows `min(index, levels - 1)` (`a2fmt::sod::Sod::lod_levels`).
    pub fn lod_index(&self, distance: f32) -> usize {
        self.lod.iter().filter(|&&l| distance < l).count()
    }
}

/// This frame's counts over the objects the strategic handoff looks at.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CullStats {
    pub objects: usize,
    pub culled: usize,
    pub fading: usize,
    /// Objects at each LOD index 0..=4.
    pub lod: [usize; 5],
}

impl CullStats {
    pub fn count(&mut self, params: &CullParams, distance: f32) {
        self.objects += 1;
        let fade = params.fade(distance);
        if fade <= 0.0 {
            self.culled += 1;
        } else if fade < 1.0 {
            self.fading += 1;
        }
        self.lod[params.lod_index(distance)] += 1;
    }
}

pub struct CullPlugin;

impl Plugin for CullPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CullParams>()
            .init_resource::<CullStats>()
            .init_resource::<RenderMarks>()
            .add_systems(Update, (load.run_if(resource_exists_and_changed::<Mounted>), mark_view.run_if(resource_changed::<CullParams>), mark_counts));
    }
}

/// The mounted files' `ART_CFG.h` (the same read `map` and `space` make).
fn load(vfs: Res<Mounted>, mut params: ResMut<CullParams>) {
    let cfg = vfs.0.read("ART_CFG.h").and_then(Result::ok).map(|b| CfgH::parse(&String::from_utf8_lossy(&b)));
    let read = CullParams::from_cfg(cfg.as_ref());
    if *params != read {
        *params = read;
    }
}

fn tick(clock: &Option<Res<AnimClock>>) -> u64 {
    clock.as_ref().map_or(0, |c| c.tick())
}

fn mark_view(params: Res<CullParams>, clock: Option<Res<AnimClock>>, mut marks: ResMut<RenderMarks>) {
    marks.0.push(Mark {
        tick: tick(&clock),
        kind: "render.view",
        fields: vec![
            ("far", Val::Text(params.far.to_string())),
            ("near", Val::Text(params.near.to_string())),
            ("cull", Val::Text(params.cull.to_string())),
            ("fade_from", Val::Text(params.fade_from.to_string())),
        ],
    });
}

fn mark_counts(stats: Res<CullStats>, clock: Option<Res<AnimClock>>, mut marks: ResMut<RenderMarks>, mut last: Local<Option<CullStats>>) {
    if *last == Some(*stats) || (last.is_none() && stats.objects == 0) {
        return;
    }
    *last = Some(*stats);
    marks.0.push(Mark {
        tick: tick(&clock),
        kind: "render.cull",
        fields: vec![
            ("objects", Val::Text(stats.objects.to_string())),
            ("culled", Val::Text(stats.culled.to_string())),
            ("fading", Val::Text(stats.fading.to_string())),
            ("lod", Val::Text(stats.lod.map(|n| n.to_string()).join(","))),
        ],
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_work_plan_numbers() {
        let p = CullParams::default();
        assert_eq!((p.cull, p.fade_from, p.far, p.near), (2800.0, 0.9, 20000.0, 20.0));
    }

    #[test]
    fn values_come_from_art_cfg_and_a_missing_key_keeps_the_default() {
        let cfg = CfgH::parse(
            "float NEAR_CLIPPING_PLANE = 35.0;\nfloat FAR_CLIPPING_PLANE = 100000.0;\nfloat cfgOBJECT_CULLING_DISTANCE = 3500.0;\nfloat DETAIL_2_LOD_0_DISTANCE = 1200.0;\n",
        );
        let p = CullParams::from_cfg(Some(&cfg));
        assert_eq!((p.cull, p.far, p.near), (3500.0, 100000.0, 35.0));
        assert_eq!(p.lod, [1200.0, 500.0, 300.0, 100.0]);
        assert_eq!(CullParams::from_cfg(None), CullParams::default());
        // A zero is a file switching the value off, not a distance.
        let zero = CfgH::parse("float cfgOBJECT_CULLING_DISTANCE = 0.0;");
        assert_eq!(CullParams::from_cfg(Some(&zero)).cull, 2800.0);
    }

    #[test]
    fn fade_runs_from_nine_tenths_to_the_distance() {
        let p = CullParams::default();
        assert_eq!(p.fade(0.0), 1.0);
        assert_eq!(p.fade(2520.0), 1.0);
        assert!((p.fade(2660.0) - 0.5).abs() < 1e-5);
        assert_eq!(p.fade(2800.0), 0.0);
        assert_eq!(p.fade(1.0e6), 0.0);
    }

    #[test]
    fn height_counts_a_quarter() {
        let eye = Vec3::new(0.0, 4000.0, 0.0);
        assert_eq!(CullParams::distance(eye, Vec3::ZERO), 1000.0);
        assert_eq!(CullParams::distance(Vec3::ZERO, Vec3::new(300.0, 0.0, 400.0)), 500.0);
    }

    #[test]
    fn lod_index_rises_as_the_camera_nears() {
        let p = CullParams::default();
        let at = |d| p.lod_index(d);
        assert_eq!([at(2000.0), at(900.0), at(400.0), at(200.0), at(50.0)], [0, 1, 2, 3, 4]);
    }

    #[test]
    fn counts_and_marks_follow_the_distances() {
        let p = CullParams::default();
        let mut s = CullStats::default();
        for d in [10.0, 2600.0, 2700.0, 3000.0] {
            s.count(&p, d);
        }
        assert_eq!((s.objects, s.culled, s.fading), (4, 1, 2));
        let mut app = App::new();
        app.add_plugins(CullPlugin);
        *app.world_mut().resource_mut::<CullStats>() = s;
        app.update();
        let lines = app.world().resource::<RenderMarks>().lines();
        assert!(lines.iter().any(|l| l == "render.view far=20000 near=20 cull=2800 fade_from=0.9"), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("render.cull objects=4 culled=1 fading=2 lod=")), "{lines:?}");
        // Unchanged counts add no line.
        let n = lines.len();
        app.update();
        assert_eq!(app.world().resource::<RenderMarks>().lines().len(), n);
    }
}
