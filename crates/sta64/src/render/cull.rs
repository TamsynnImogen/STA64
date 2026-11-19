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
//! Two more limits belong to the view, and one script can move each:
//!
//! | value | where it comes from | applies |
//! |---|---|---|
//! | object culling distance | `cfgOBJECT_CULLING_DISTANCE`; Armada II's `SetObjectCullingDistance` replaces it and `RestoreObjectCullingDistance` puts the `ART_CFG.h` value back | in every view (read from the executable, static), not only in a cineractive; Armada I has neither key nor calls |
//! | cineractive far plane | a value of the game itself, not of the file ([`CINERACTIVE_FAR_A2`], Armada I [`CINERACTIVE_FAR_A1`]); Armada II's `SetClippingDistance` replaces it | while a cineractive runs (read from the executable, static) |
//!
//! `ART_CFG.h` also has a key `CINERACTIVE_FAR_CLIPPING_PLANE`: it has no effect in the games
//! (read from the executable, static), so STA64 does not read it either. The script calls reach the client as
//! presentation events ([`ScriptView`]); they are not simulation state, so no hash, snapshot or
//! replay holds them.
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

/// The far plane of a cineractive in Armada II and the Fleet Operations family: the game's own
/// value, which no `ART_CFG.h` key sets (read from the executable, static).
pub const CINERACTIVE_FAR_A2: f32 = 20000.0;
/// The same for Armada I (read from the executable, static; taken to be in the units of its
/// `FAR_CLIPPING_PLANE`, not measured).
pub const CINERACTIVE_FAR_A1: f32 = 1000.0;
/// STA64's own presentation value, not game data: in normal play a model turns into its map
/// symbol at this multiple of the culling distance (the originals draw nothing there). Was 1.0.
/// The distance is the engine's (euclidean, the vertical scaled by a quarter), so a point at
/// the edge of a wide view is farther than one at the centre at the same depth; the multiple
/// moves both a little later.
pub const SYMBOL_DISTANCE_FACTOR: f32 = 1.25;

/// The distances in force.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CullParams {
    /// Past this an object's model is not drawn: a strategic object becomes its symbol.
    pub cull: f32,
    /// Share of `cull` where the fade starts.
    pub fade_from: f32,
    pub far: f32,
    pub near: f32,
    /// Camera distance inside which the next, finer LOD shows.
    pub lod: [f32; 4],
    /// The file states an object culling distance (Armada II family). Armada I does not and
    /// culls no object by distance.
    pub object_culling: bool,
    /// The far plane while a cineractive runs (the game's own value; a script call moves it).
    pub cineractive_far: f32,
}

impl Default for CullParams {
    fn default() -> Self {
        CullParams { cull: DEFAULT_CULL, fade_from: FADE_FROM, far: DEFAULT_FAR, near: DEFAULT_NEAR, lod: DEFAULT_LOD, object_culling: true, cineractive_far: CINERACTIVE_FAR_A2 }
    }
}

impl CullParams {
    /// Read `ART_CFG.h`; a key it lacks (or sets to nothing useful) keeps the default.
    pub fn from_cfg(cfg: Option<&CfgH>) -> CullParams {
        let d = CullParams::default();
        let Some(c) = cfg else { return d };
        let pos = |k: &str, v: f32| c.f32(k).filter(|x| *x > 0.0).unwrap_or(v);
        // A file with LOD keys states its own levels: a level it lacks (Armada I has
        // three) is no level, not A2's default. A file with none keeps the defaults.
        let mut lod = [0.0; 4];
        let mut any = false;
        for (i, l) in lod.iter_mut().enumerate() {
            if let Some(v) = c.f32(&format!("DETAIL_2_LOD_{i}_DISTANCE")).filter(|x| *x > 0.0) {
                *l = v;
                any = true;
            }
        }
        if !any {
            lod = d.lod;
        }
        let far = pos("FAR_CLIPPING_PLANE", d.far);
        // No culling key (Armada I): nothing is drawn past the far plane either.
        let cull = pos("cfgOBJECT_CULLING_DISTANCE", d.cull.min(far));
        let object_culling = c.f32("cfgOBJECT_CULLING_DISTANCE").is_some_and(|x| x > 0.0);
        let cineractive_far = if object_culling { CINERACTIVE_FAR_A2 } else { CINERACTIVE_FAR_A1 };
        CullParams { cull, fade_from: d.fade_from, far, near: pos("NEAR_CLIPPING_PLANE", d.near), lod, object_culling, cineractive_far }
    }

    /// These limits with a script's changes applied.
    pub fn with_script(&self, script: &ScriptView) -> CullParams {
        CullParams { cull: script.cull.unwrap_or(self.cull), cineractive_far: script.far.unwrap_or(self.cineractive_far), ..*self }
    }

    /// The distances for the model-to-symbol handoff of normal play: the culling distance
    /// scaled by [`SYMBOL_DISTANCE_FACTOR`], the fade band moving with it.
    pub fn for_symbols(&self) -> CullParams {
        CullParams { cull: self.cull * SYMBOL_DISTANCE_FACTOR, ..*self }
    }

    /// Whether a cineractive draws no model of this object: it lies wholly past the cineractive
    /// far plane (`depth` along the view minus `radius`; the sphere test is Refit's reading of the
    /// engine, not measured for the exact measure), or, where the game culls by distance, past
    /// the culling distance. The fade before it (0.9 of the distance onwards) is not drawn: the
    /// model is whole until it goes.
    pub fn cineractive_hides(&self, distance: f32, depth: f32, radius: f32) -> bool {
        depth - radius > self.cineractive_far || (self.object_culling && self.fade(distance) <= 0.0)
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

/// What the mission scripts of Armada II set for the view: the object culling distance
/// (`SetObjectCullingDistance`, back to the file's with `RestoreObjectCullingDistance`) and the
/// cineractive far plane (`SetClippingDistance`). Presentation state of the client, read from the
/// events the scripts raise; nothing of it is in the simulation, a snapshot or a replay. It is
/// cleared when a mission starts, ends or restarts in place; a save does not hold it, so a
/// mission loaded from a save has the file's distance until its script sets one again.
#[derive(Resource, Default, Clone, Copy, PartialEq, Debug)]
pub struct ScriptView {
    pub cull: Option<f32>,
    pub far: Option<f32>,
}

impl ScriptView {
    /// Takes a presentation call by its name and arguments; false when it is not one of the three.
    /// A value that is not a positive number is ignored, as the file's keys are.
    pub fn apply(&mut self, name: &str, value: Option<f32>) -> bool {
        let positive = value.filter(|v| v.is_finite() && *v > 0.0);
        match name {
            "A2.SetObjectCullingDistance" => self.cull = positive.or(self.cull),
            "A2.RestoreObjectCullingDistance" => self.cull = None,
            "A2.SetClippingDistance" => self.far = positive.or(self.far),
            _ => return false,
        }
        true
    }
}

/// The limits in force with a script's changes, for a system with many parameters.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ViewLimits<'w> {
    file: Res<'w, CullParams>,
    script: Res<'w, ScriptView>,
}

impl ViewLimits<'_> {
    pub fn get(&self) -> CullParams {
        self.file.with_script(&self.script)
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
            .init_resource::<ScriptView>()
            .init_resource::<CullStats>()
            .init_resource::<RenderMarks>()
            .add_systems(OnEnter(crate::app::AppState::Loading), forget_script_view)
            .add_systems(OnExit(crate::app::AppState::InGame), forget_script_view)
            .add_systems(Update, (load.run_if(resource_exists_and_changed::<Mounted>), mark_view.run_if(|file: Res<CullParams>, script: Res<ScriptView>| file.is_changed() || script.is_changed()), mark_counts));
    }
}

/// A mission's script settings end with it.
fn forget_script_view(mut view: ResMut<ScriptView>) {
    *view = ScriptView::default();
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

/// `cull=` is the culling distance in force (the file's, or the script's while one is set); the
/// symbol handoff of normal play lies at [`SYMBOL_DISTANCE_FACTOR`] times it, and `render.cull`
/// counts against the distance printed here.
fn mark_view(file: Res<CullParams>, script: Res<ScriptView>, clock: Option<Res<AnimClock>>, mut marks: ResMut<RenderMarks>) {
    let params = file.with_script(&script);
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
    // One line, replaced as the counts change: a long match must not grow the marks.
    if marks.0.last().is_some_and(|m| m.kind == "render.cull") {
        marks.0.pop();
    }
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
        assert_eq!(p.lod, [1200.0, 0.0, 0.0, 0.0], "a file's own levels, not A2's for the ones it lacks");
        // Armada I: no culling key, a short far plane.
        let a1 = CfgH::parse("float FAR_CLIPPING_PLANE = 1400.0;");
        assert_eq!(CullParams::from_cfg(Some(&a1)).cull, 1400.0);
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
    fn the_games_view_limits_come_from_the_file_and_the_executable() {
        let a2 = CullParams::from_cfg(Some(&CfgH::parse("float cfgOBJECT_CULLING_DISTANCE = 1234.0;\nfloat CINERACTIVE_FAR_CLIPPING_PLANE = 4321.0;\n")));
        assert!(a2.object_culling);
        assert_eq!(a2.cineractive_far, CINERACTIVE_FAR_A2, "the file's key of that name is not read");
        let a1 = CullParams::from_cfg(Some(&CfgH::parse("float FAR_CLIPPING_PLANE = 777.0;\nfloat CINERACTIVE_FAR_CLIPPING_PLANE = 555.0;\n")));
        assert!(!a1.object_culling);
        assert_eq!(a1.cineractive_far, CINERACTIVE_FAR_A1);
    }

    #[test]
    fn script_calls_replace_the_culling_distance_and_the_cineractive_far_plane() {
        let mut v = ScriptView::default();
        let p = CullParams::default();
        assert_eq!(p.with_script(&v), p);
        assert!(v.apply("A2.SetObjectCullingDistance", Some(1500.0)));
        assert!(v.apply("A2.SetClippingDistance", Some(4000.0)));
        assert!(!v.apply("A2.MuteSounds", Some(1.0)));
        let s = p.with_script(&v);
        assert_eq!((s.cull, s.cineractive_far, s.far), (1500.0, 4000.0, p.far));
        assert!(v.apply("A2.RestoreObjectCullingDistance", None));
        assert_eq!(p.with_script(&v).cull, p.cull);
        assert_eq!(p.with_script(&v).cineractive_far, 4000.0);
    }

    #[test]
    fn a_cineractive_hides_past_the_far_plane_and_where_the_game_culls_past_the_distance() {
        // Authored limits: culling distance 1000, cineractive far plane 5000.
        let a2 = CullParams { cull: 1000.0, cineractive_far: 5000.0, ..CullParams::default() };
        // Inside both: whole, even in the fade band (the fade is not drawn).
        assert!(!a2.cineractive_hides(950.0, 100.0, 10.0));
        assert!(a2.cineractive_hides(1000.0, 100.0, 10.0));
        // Past the far plane by its whole sphere only.
        assert!(!a2.cineractive_hides(100.0, 5005.0, 10.0));
        assert!(a2.cineractive_hides(100.0, 5011.0, 10.0));
        // A game that culls no object by distance (Armada I): only its far plane.
        let a1 = CullParams { object_culling: false, cineractive_far: 400.0, ..a2 };
        assert!(!a1.cineractive_hides(1.0e6, 390.0, 10.0));
        assert!(a1.cineractive_hides(50.0, 411.0, 10.0));
        // The symbol handoff of normal play comes later than the culling distance.
        let sym = a2.for_symbols();
        assert_eq!(sym.cull, a2.cull * SYMBOL_DISTANCE_FACTOR);
        assert_eq!(sym.fade(a2.cull), 1.0);
        assert_eq!(sym.fade(sym.cull), 0.0);
    }

    /// The match's end and the next match's loading drop what a script set.
    #[test]
    fn script_settings_end_with_the_mission() {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin).init_state::<crate::app::AppState>().add_plugins(CullPlugin);
        app.update();
        let set = |app: &mut App| *app.world_mut().resource_mut::<ScriptView>() = ScriptView { cull: Some(1000.0), far: Some(2000.0) };
        for to in [crate::app::AppState::InGame, crate::app::AppState::FrontEnd] {
            set(&mut app);
            app.world_mut().resource_mut::<NextState<crate::app::AppState>>().set(to);
            app.update();
            app.update();
            if to == crate::app::AppState::FrontEnd {
                assert_eq!(*app.world().resource::<ScriptView>(), ScriptView::default(), "leaving the match");
            }
        }
        set(&mut app);
        app.world_mut().resource_mut::<NextState<crate::app::AppState>>().set(crate::app::AppState::Loading);
        app.update();
        app.update();
        assert_eq!(*app.world().resource::<ScriptView>(), ScriptView::default(), "loading a mission");
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
