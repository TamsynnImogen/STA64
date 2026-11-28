//! Safe mode: a render pipeline that fails validation does not end a play session (#1418).
//!
//! The engine's default answer to any render error is to quit the application. In a play build [`install`] replaces
//! it with [`handler`]:
//! - **A validation error** (a pipeline that cannot be built, such as a mesh missing an attribute its shader reads) is
//!   logged once with the engine's own message, which names the pipeline; one notice tells the player; every stock
//!   effect (everything drawn with the stock shot material) is hidden from then on, and rendering goes on. Further
//!   validation errors in the same session are counted: the second one is logged, the count is logged at exit, and no
//!   further notice is raised.
//! - **Every other error** (out of memory, device lost, internal) keeps the default: the application quits.
//!
//! What a player loses in safe mode: the stock look's weapon shots, sparks, impact flares, shield caps, shields-down
//! bands and explosions are not drawn for the rest of the session; the game itself goes on. The session's log names the
//! pipeline. The notice is the player's only clue, so it also says that restarting the game switches the effects back on
//! and that the log names what failed.
//!
//! Tests and the bench do not install the handler (pinned): there a render error stays fatal, so a defect cannot pass
//! unnoticed.

use bevy::prelude::*;
use bevy::render::error_handler::{ErrorType, RenderError, RenderErrorHandler, RenderErrorPolicy};

use crate::render::fx::stock_shots::StockShotMaterial;
use crate::ui::i18n::tr;
use crate::ui::notices::{Notice, Severity};

/// What to do about one render error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Quit, as the engine does by default.
    Quit,
    /// The first validation error of the session: switch safe mode on, with a notice.
    SwitchOn,
    /// A later validation error: count it and go on.
    Count,
}

/// The decision for an error of kind `ty`, with safe mode already `on` or not (pure).
pub fn decide(ty: &ErrorType, on: bool) -> Decision {
    match (ty, on) {
        (ErrorType::Validation, false) => Decision::SwitchOn,
        (ErrorType::Validation, true) => Decision::Count,
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
        std::mem::take(&mut self.notice_due).then(|| Notice::new(Severity::Warn, tr("An effect could not be drawn; stock effects are switched off for this session. Restarting the game switches them back on; the log names what failed.")))
    }
}

/// The engine's hook: decide per error kind. Plain function, as the engine wants.
pub fn handler(error: &RenderError, main: &mut World, _render: &mut World) -> RenderErrorPolicy {
    let on = main.get_resource::<SafeMode>().is_some_and(|s| s.on);
    match decide(&error.ty, on) {
        Decision::Quit => {
            error!("Quitting the application due to {:?} RenderError: {}", error.ty, error.description);
            main.write_message(AppExit::error());
            RenderErrorPolicy::StopRendering
        }
        Decision::SwitchOn => {
            error!("Safe mode: a render pipeline failed validation, stock effects are switched off for this session: {}", error.description);
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

/// Install the handler on the play app (never on tests or the bench).
pub fn install(app: &mut App) {
    app.insert_resource(RenderErrorHandler(handler)).add_plugins(SafeModePlugin);
}

/// The resource, the notice, the hiding of the stock effects and the count at exit.
pub struct SafeModePlugin;

impl Plugin for SafeModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SafeMode>()
            .add_message::<Notice>()
            .add_systems(Update, raise_notice)
            .add_systems(PostUpdate, hide_stock_effects.before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate))
            .add_systems(Last, log_count_at_exit);
        #[cfg(debug_assertions)]
        app.add_systems(PostUpdate, guard_stock_meshes);
    }
}

/// The vertex attributes the stock shot material's fragment stage reads, by name, that `mesh` lacks.
#[cfg(debug_assertions)]
fn missing_vertex_inputs(mesh: &Mesh) -> Vec<&'static str> {
    [
        ("position", Mesh::ATTRIBUTE_POSITION.id),
        ("normal", Mesh::ATTRIBUTE_NORMAL.id),
        ("uv", Mesh::ATTRIBUTE_UV_0.id),
        ("colour", Mesh::ATTRIBUTE_COLOR.id),
    ]
    .into_iter()
    .filter(|(_, id)| mesh.attribute(*id).is_none())
    .map(|(name, _)| name)
    .collect()
}

/// The guard (debug builds and tests only: it does not exist in a release build, so it costs nothing there): a visible
/// entity drawn with the stock shot material whose mesh lacks an attribute the material's shader reads is a defect that
/// would end the game in a build without safe mode (#1418). Every scene of the client's tests that draws a stock effect
/// runs this on every such entity, so a future mesh handed to the material without its colour fails the scene.
#[cfg(debug_assertions)]
fn guard_stock_meshes(meshes: Option<Res<Assets<Mesh>>>, q: Query<(Entity, &Mesh3d, &Visibility), With<MeshMaterial3d<StockShotMaterial>>>) {
    let Some(meshes) = meshes else { return };
    for (e, mesh, vis) in &q {
        if *vis == Visibility::Hidden {
            continue;
        }
        // A batch mesh is empty (and hidden) until its first fill.
        let Some(m) = meshes.get(&mesh.0).filter(|m| m.count_vertices() > 0) else { continue };
        let missing = missing_vertex_inputs(m);
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
    fn only_validation_errors_are_survived_and_the_first_switches_safe_mode_on() {
        assert_eq!(decide(&ErrorType::Validation, false), Decision::SwitchOn);
        assert_eq!(decide(&ErrorType::Validation, true), Decision::Count);
        for ty in [ErrorType::OutOfMemory, ErrorType::Internal, ErrorType::DeviceLost] {
            assert_eq!(decide(&ty, false), Decision::Quit, "{ty:?}");
            assert_eq!(decide(&ty, true), Decision::Quit, "{ty:?} in safe mode");
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

    /// Tests and the bench do not install the handler: only the play path of the library does.
    #[test]
    fn only_the_play_path_installs_the_handler() {
        let lib = include_str!("../lib.rs");
        assert_eq!(lib.matches("safe_mode::install").count(), 1);
        for other in [include_str!("../bench/mod.rs"), include_str!("../bench/frames.rs")] {
            assert!(!other.contains("safe_mode"), "the bench keeps a render error fatal");
        }
    }
}
