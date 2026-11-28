//! Safe mode: a render pipeline that fails validation does not end a play session (#1418).
//!
//! The engine's default answer to any render error is to quit the application. In a play session ([`plays`]: the front
//! end, a skirmish or a map, with no control channel and no MCP seat) [`install`] replaces it with [`handler`]:
//! - **A validation error** (a pipeline that cannot be built, such as a mesh missing an attribute its shader reads) is
//!   logged once with the engine's own message, which names the pipeline; one notice tells the player; every stock
//!   effect (everything drawn with the stock shot material) is hidden from then on, and rendering goes on. The frame
//!   with the error and the one after it are lost (one flicker) before the hiding takes effect.
//! - **Bounded:** the hiding removes the cause only if the failing pipeline is a stock effect's. If validation errors
//!   still arrive more than [`GRACE_FRAMES`] frames after safe mode was switched on, the cause is not the stock effects:
//!   the handler quits as the engine's own does, with a log line saying so. Ignoring an error whose cause stays would
//!   give a broken or strobing picture and a flooded log.
//! - **Further validation errors** inside the grace: counted; the second is logged, the count is logged at exit, and
//!   there is no further notice.
//! - **Every other error** (out of memory, device lost, internal) keeps the default: the application quits.
//!
//! What a player loses in safe mode: the stock look's weapon shots (weapon fire becomes invisible under the Stock look),
//! sparks, impact flares, shield caps, shields-down bands and explosions for the rest of the session; the game itself
//! goes on. The notice does not claim a cause it cannot know: it says an effect could not be drawn and the stock effects
//! were switched off as a precaution; restarting the game switches them back on and the log names what failed.
//!
//! The debug guard ([`guard_stock_meshes`]) is registered by the stock effects' own plugin in debug builds and tests only:
//! there it panics at a mesh without an attribute the material's shader reads, where a release build would enter safe
//! mode. Tests, the bench, the viewer and control sessions do not install the handler (pinned per launch mode): there a
//! render error stays fatal.

use bevy::prelude::*;
use bevy::render::error_handler::{ErrorType, RenderError, RenderErrorHandler, RenderErrorPolicy};

use crate::render::fx::stock_shots::StockShotMaterial;
use crate::ui::i18n::tr;
use crate::ui::notices::{Notice, Severity};

/// Frames after safe mode was switched on during which validation errors may still arrive from the error already in
/// flight: the engine polls an error once per frame at extraction and the hiding lands at the following frame's
/// post-update (two frames), plus a margin of four. Errors beyond it are not from the stock effects.
pub const GRACE_FRAMES: u32 = 6;

/// What to do about one render error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Quit, as the engine does by default.
    Quit,
    /// The first validation error of the session: switch safe mode on, with a notice.
    SwitchOn,
    /// A later validation error inside the grace: count it and go on.
    Count,
    /// A validation error that keeps coming after the grace: not the stock effects' pipeline: quit.
    QuitUnbounded,
}

/// The decision for an error of kind `ty`; `since`: the frames since safe mode was switched on, `None` while it is off
/// (pure).
pub fn decide(ty: &ErrorType, since: Option<u32>) -> Decision {
    match (ty, since) {
        (ErrorType::Validation, None) => Decision::SwitchOn,
        (ErrorType::Validation, Some(f)) if f <= GRACE_FRAMES => Decision::Count,
        (ErrorType::Validation, Some(_)) => Decision::QuitUnbounded,
        _ => Decision::Quit,
    }
}

/// The session's safe-mode state (the main world).
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct SafeMode {
    /// Stock effects are switched off.
    pub on: bool,
    /// Validation errors met in safe mode, the first included.
    pub errors: u32,
    /// Frames since safe mode was switched on.
    pub frames: u32,
    notice_due: bool,
}

impl SafeMode {
    /// Take one error of validation kind in: true when it is the one that switches safe mode on.
    pub fn validation_error(&mut self) -> bool {
        self.errors += 1;
        let first = !self.on;
        if first {
            self.on = true;
            self.notice_due = true;
        }
        first
    }

    /// The notice the first error asked for, once.
    pub fn take_notice(&mut self) -> Option<Notice> {
        std::mem::take(&mut self.notice_due).then(|| Notice::new(Severity::Warn, tr("An effect could not be drawn; stock effects are switched off as a precaution. Restarting the game switches them back on; the log names what failed.")))
    }
}

/// The engine's hook: decide per error kind. Plain function, as the engine wants.
pub fn handler(error: &RenderError, main: &mut World, _render: &mut World) -> RenderErrorPolicy {
    let since = main.get_resource::<SafeMode>().filter(|s| s.on).map(|s| s.frames);
    match decide(&error.ty, since) {
        Decision::Quit => {
            error!("Quitting the application due to {:?} RenderError: {}", error.ty, error.description);
            main.write_message(AppExit::error());
            RenderErrorPolicy::StopRendering
        }
        Decision::QuitUnbounded => {
            error!("Quitting the application: validation errors keep coming after the stock effects were switched off, so the cause is not the stock effects: {}", error.description);
            main.write_message(AppExit::error());
            RenderErrorPolicy::StopRendering
        }
        Decision::SwitchOn => {
            error!("Safe mode: a render pipeline failed validation, stock effects are switched off as a precaution: {}", error.description);
            main.get_resource_or_insert_with(SafeMode::default).validation_error();
            RenderErrorPolicy::Ignore
        }
        Decision::Count => {
            let errors = {
                let mut s = main.get_resource_or_insert_with(SafeMode::default);
                s.validation_error();
                s.errors
            };
            if errors == 2 {
                error!("Safe mode: further validation errors follow ({})", error.description);
            }
            RenderErrorPolicy::Ignore
        }
    }
}

/// Whether a launch is a play session, the only one that survives a failed pipeline: the front end, a skirmish or a
/// map, with no control channel and no MCP seat. Not the viewer, a bench scene or a control or MCP session.
pub fn plays(cfg: &crate::app::LaunchConfig) -> bool {
    use crate::app::Mode;
    matches!(cfg.mode(), Mode::FrontEnd | Mode::Game | Mode::Map) && cfg.control.is_none() && cfg.mcp_seat.is_none() && cfg.mcp_endpoint.is_none()
}

/// Install the handler on the play app (see [`plays`]; never on tests, the bench or the viewer).
pub fn install(app: &mut App) {
    app.insert_resource(RenderErrorHandler(handler)).add_plugins(SafeModePlugin);
}

/// The resource, the notice, the hiding of the stock effects and the count at exit.
pub struct SafeModePlugin;

impl Plugin for SafeModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SafeMode>()
            .add_message::<Notice>()
            .add_systems(First, count_frames)
            .add_systems(Update, raise_notice)
            .add_systems(PostUpdate, hide_stock_effects.before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate))
            .add_systems(Last, log_count_at_exit);
    }
}

fn count_frames(mut safe: ResMut<SafeMode>) {
    if safe.on {
        safe.frames = safe.frames.saturating_add(1);
    }
}

/// Marks that the mesh guard is registered on an app, so the plugins that all call [`register_mesh_guard`] add it once.
#[derive(Resource)]
struct MeshGuardInstalled;

/// Register the debug mesh guard ([`guard_stock_meshes`]) on `app`: called by every plugin that attaches a mesh to the
/// stock shot material (the shots, the explosions, the shield hits, the shields-down bands, the material plugin) and by the
/// tests that add those systems without a plugin. Safe to call any number of times; does nothing in a release build.
pub(crate) fn register_mesh_guard(app: &mut App) {
    #[cfg(debug_assertions)]
    if !app.world().contains_resource::<MeshGuardInstalled>() {
        app.insert_resource(MeshGuardInstalled).add_systems(PostUpdate, guard_stock_meshes);
    }
    #[cfg(not(debug_assertions))]
    let _ = app;
}

/// The vertex attributes the stock shot material's fragment stage reads, by name, that `mesh` lacks; `None` when the
/// mesh's vertex data is no longer readable (it was handed to the renderer: the safe forms of the engine's accessors
/// return an error then, where the plain ones panic: #1426).
#[cfg(debug_assertions)]
fn missing_vertex_inputs(mesh: &Mesh) -> Option<Vec<&'static str>> {
    let mut missing = Vec::new();
    for (name, id) in [
        ("position", Mesh::ATTRIBUTE_POSITION.id),
        ("normal", Mesh::ATTRIBUTE_NORMAL.id),
        ("uv", Mesh::ATTRIBUTE_UV_0.id),
        ("colour", Mesh::ATTRIBUTE_COLOR.id),
    ] {
        if mesh.try_attribute_option(id).ok()?.is_none() {
            missing.push(name);
        }
    }
    Some(missing)
}

/// The guard (debug builds and tests only: it does not exist in a release build, so it costs nothing there; registered
/// by the stock effects' own plugins, so every scene that draws a stock effect runs it): a visible entity drawn with the
/// stock shot material whose mesh lacks an attribute the material's shader reads is a defect that would end the game in
/// a build without safe mode (#1418).
///
/// It reads only through the safe accessors and never touches a mesh whose data was handed to the renderer: those are
/// skipped for good (it remembers only those). So in a real play session it checks a mesh on the frames before the
/// renderer takes it (a new effect mesh is usually visible, and so checked, in the frame it appears); in the tests,
/// which never extract, it checks every visible entity every frame, so a batch mesh refilled each frame is checked on
/// each fill. It can panic for no reason other than its own finding.
#[cfg(debug_assertions)]
pub(crate) fn guard_stock_meshes(meshes: Option<Res<Assets<Mesh>>>, q: Query<(Entity, &Mesh3d, &Visibility), With<MeshMaterial3d<StockShotMaterial>>>, mut taken: Local<std::collections::HashSet<AssetId<Mesh>>>) {
    let Some(meshes) = meshes else { return };
    for (e, mesh, vis) in &q {
        if *vis == Visibility::Hidden || taken.contains(&mesh.0.id()) {
            continue;
        }
        let Some(m) = meshes.get(&mesh.0) else { continue };
        // A mesh already in the renderer cannot be read: skipped for good. A batch mesh is empty (and hidden) until its
        // first fill: looked at again.
        match m.try_attribute_option(Mesh::ATTRIBUTE_POSITION.id) {
            Err(_) => {
                taken.insert(mesh.0.id());
                continue;
            }
            Ok(None) => continue,
            Ok(Some(positions)) if positions.is_empty() => continue,
            Ok(Some(_)) => {}
        }
        let Some(missing) = missing_vertex_inputs(m) else { continue };
        assert!(missing.is_empty(), "entity {e:?} is drawn with the stock shot material but its mesh lacks {missing:?}: the pipeline would fail validation");
    }
}

fn raise_notice(mut safe: ResMut<SafeMode>, mut out: MessageWriter<Notice>) {
    if safe.notice_due
        && let Some(n) = safe.take_notice()
    {
        out.write(n);
    }
}

/// Safe mode: everything drawn with the stock shot material is hidden (every frame: the effects set their own
/// visibility each frame).
fn hide_stock_effects(safe: Res<SafeMode>, mut q: Query<&mut Visibility, With<MeshMaterial3d<StockShotMaterial>>>) {
    if !safe.on {
        return;
    }
    for mut v in &mut q {
        v.set_if_neq(Visibility::Hidden);
    }
}

fn log_count_at_exit(mut exits: MessageReader<AppExit>, safe: Res<SafeMode>) {
    if exits.read().next().is_some() && safe.errors > 1 {
        error!("Safe mode: {} validation errors in this session", safe.errors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Got(Vec<Notice>);

    #[test]
    fn only_validation_errors_are_survived_inside_the_grace_and_the_first_switches_safe_mode_on() {
        assert_eq!(decide(&ErrorType::Validation, None), Decision::SwitchOn);
        assert_eq!(decide(&ErrorType::Validation, Some(0)), Decision::Count);
        assert_eq!(decide(&ErrorType::Validation, Some(GRACE_FRAMES)), Decision::Count);
        assert_eq!(decide(&ErrorType::Validation, Some(GRACE_FRAMES + 1)), Decision::QuitUnbounded, "the cause is not the stock effects");
        for ty in [ErrorType::OutOfMemory, ErrorType::Internal, ErrorType::DeviceLost] {
            assert_eq!(decide(&ty, None), Decision::Quit, "{ty:?}");
            assert_eq!(decide(&ty, Some(1)), Decision::Quit, "{ty:?} in safe mode");
        }
    }

    #[test]
    fn the_first_error_raises_one_notice_and_later_ones_only_count() {
        let mut s = SafeMode::default();
        assert!(s.validation_error());
        assert!(s.on);
        assert!(s.take_notice().is_some());
        assert!(!s.validation_error() && !s.validation_error());
        assert!(s.take_notice().is_none(), "no further notice");
        assert_eq!(s.errors, 3);
    }

    #[test]
    fn the_handler_goes_on_after_a_validation_error_and_quits_after_any_other() {
        let mut main = World::new();
        main.init_resource::<SafeMode>();
        let mut render = World::new();
        let err = |ty| RenderError { ty, description: "a pipeline failed".into(), source: None };
        assert!(matches!(handler(&err(ErrorType::Validation), &mut main, &mut render), RenderErrorPolicy::Ignore));
        assert!(main.resource::<SafeMode>().on);
        assert!(matches!(handler(&err(ErrorType::Validation), &mut main, &mut render), RenderErrorPolicy::Ignore));
        assert_eq!(main.resource::<SafeMode>().errors, 2);
        assert!(matches!(handler(&err(ErrorType::OutOfMemory), &mut main, &mut render), RenderErrorPolicy::StopRendering));
        // Errors that keep coming after the grace are not the stock effects': quit, with the exit message.
        main.resource_mut::<SafeMode>().frames = GRACE_FRAMES + 1;
        assert!(matches!(handler(&err(ErrorType::Validation), &mut main, &mut render), RenderErrorPolicy::StopRendering));
    }

    #[test]
    fn the_frame_count_runs_only_in_safe_mode() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).add_plugins(SafeModePlugin).init_asset::<StockShotMaterial>().init_asset::<Image>();
        app.update();
        assert_eq!(app.world().resource::<SafeMode>().frames, 0);
        app.world_mut().resource_mut::<SafeMode>().validation_error();
        app.update();
        app.update();
        assert_eq!(app.world().resource::<SafeMode>().frames, 2);
    }

    #[test]
    fn safe_mode_hides_the_stock_effects_raises_one_notice_and_leaves_other_things() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).add_plugins(SafeModePlugin).init_asset::<StockShotMaterial>().init_asset::<Image>().init_resource::<Got>();
        app.add_systems(Update, (|mut r: MessageReader<Notice>, mut got: ResMut<Got>| got.0.extend(r.read().cloned())).after(raise_notice));
        let material = app.world_mut().resource_mut::<Assets<StockShotMaterial>>().add(StockShotMaterial { params: Default::default(), texture: Handle::default() });
        let stock = app.world_mut().spawn((MeshMaterial3d(material), Visibility::Inherited)).id();
        let other = app.world_mut().spawn(Visibility::Inherited).id();
        app.update();
        assert_eq!(*app.world().get::<Visibility>(stock).unwrap(), Visibility::Inherited, "not on: untouched");
        app.world_mut().resource_mut::<SafeMode>().validation_error();
        app.update();
        app.update();
        assert_eq!(*app.world().get::<Visibility>(stock).unwrap(), Visibility::Hidden);
        assert_eq!(*app.world().get::<Visibility>(other).unwrap(), Visibility::Inherited);
        app.update();
        assert_eq!(app.world().resource::<Got>().0.len(), 1, "one notice, once");
    }

    #[cfg(debug_assertions)]
    fn guarded_app(mesh: Mesh) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).add_plugins(SafeModePlugin).init_asset::<StockShotMaterial>().init_asset::<Mesh>().init_asset::<Image>();
        // The stock effects' own plugin registers the guard in a scene; here it is added by hand.
        register_mesh_guard(&mut app);
        let material = app.world_mut().resource_mut::<Assets<StockShotMaterial>>().add(StockShotMaterial { params: Default::default(), texture: Handle::default() });
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(material), Visibility::Inherited));
        app
    }

    fn quad(colour: bool) -> Mesh {
        let m = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 3])
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 3])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0; 2]; 3]);
        if colour { m.with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 3]) } else { m }
    }

    /// #1426: a mesh the renderer has taken (its vertex data is in the render world) is never read: the guard skips it,
    /// where the plain accessors panic. The engine's own call puts the mesh in that state.
    #[cfg(debug_assertions)]
    #[test]
    fn the_guard_does_not_read_a_mesh_the_renderer_has_taken() {
        let render_only = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 3])
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 3])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0; 2]; 3]);
        let mut app = guarded_app(render_only);
        let handle = app.world().resource::<Assets<Mesh>>().iter().next().map(|(id, _)| id).expect("the mesh");
        app.world_mut().resource_mut::<Assets<Mesh>>().get_mut(handle).unwrap().take_gpu_data().expect("the engine's extraction");
        assert!(app.world().resource::<Assets<Mesh>>().get(handle).unwrap().try_attribute_option(Mesh::ATTRIBUTE_POSITION.id).is_err(), "the mesh is in the extracted state");
        // Frames after the extraction: no panic (and, colourless as it is, no finding either: it can no longer be read).
        for _ in 0..3 {
            app.update();
        }
    }

    /// A mesh that passed is looked at again: a batch mesh is refilled every frame, and a fill that loses an attribute
    /// is a defect on any frame, not only the first.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "lacks")]
    fn the_guard_checks_a_mesh_again_after_it_passed() {
        let mut app = guarded_app(quad(true));
        app.update();
        let id = app.world().resource::<Assets<Mesh>>().iter().next().map(|(id, _)| id).expect("the mesh");
        app.world_mut().resource_mut::<Assets<Mesh>>().get_mut(id).unwrap().remove_attribute(Mesh::ATTRIBUTE_COLOR);
        app.update();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_guard_passes_a_complete_mesh() {
        guarded_app(quad(true)).update();
    }

    /// The scene-level guard of #1418: a mesh without the colour, handed to the stock shot material, fails the scene.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "lacks")]
    fn the_guard_fails_a_scene_that_draws_the_material_with_a_mesh_without_the_colour() {
        guarded_app(quad(false)).update();
    }

    /// Only a play session installs the handler: per launch mode, with and without a control channel or an MCP seat.
    #[test]
    fn only_a_play_session_installs_the_handler() {
        use crate::app::{Control, LaunchConfig};
        let parse = |args: &[&str]| LaunchConfig::parse(args.iter().map(|a| a.to_string()));
        assert!(plays(&LaunchConfig::default()), "the front end (no flags)");
        assert!(plays(&parse(&["--map", "a_map"])), "a map");
        assert!(plays(&parse(&["--map", "a_map", "--skirmish"])), "a skirmish");
        assert!(!plays(&parse(&["--view", "a_model"])), "the viewer");
        assert!(!plays(&parse(&["--scene", "x.ini"])), "a bench scene");
        assert!(!plays(&parse(&["--map", "a_map", "--headless"])), "a headless run");
        assert!(!plays(&parse(&["--sim-only"])), "a sim-only run");
        let mut control = LaunchConfig::default();
        control.control = Some(Control::Stdio);
        assert!(!plays(&control), "a control session");
        let mut mcp = LaunchConfig::default();
        mcp.mcp_seat = Some(0);
        assert!(!plays(&mcp), "an MCP seat");
    }
}
