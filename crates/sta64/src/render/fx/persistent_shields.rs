//! Optional, persistent shield outline from a resolved `alwaysShowShields` ODF key.
//! This presentation effect is separate from the transient impact effects.

use std::collections::{BTreeMap, HashMap};

use bevy::asset::{AssetPath, embedded_asset};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use sta_core::ContentKey;
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::{EntityId, UnitView};

use crate::app::{AppState, GameSet};
use crate::render::overlays::ModelBounds;
use crate::render::units::{LocalPlayer, SelectRadius};
use crate::sim_link::{ContentRes, SimEntities, SimLink};

const FALLBACK_RADIUS: f32 = 20.0;
const PAD: f32 = 1.12;

#[derive(Clone, Copy, Default, ShaderType, Debug)]
pub struct OutlineParams {
    pub eye: Vec4,
}

/// One shared material for all persistent outlines. Its camera value changes,
/// never its count; impact materials remain wholly independent.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct PersistentShieldMaterial {
    #[uniform(0)]
    pub params: OutlineParams,
}

impl Material for PersistentShieldMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::parse("embedded://sta64/render/fx/persistent_shields.wesl").into_owned(),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None::<Face>;
        Ok(())
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct PersistentShield(#[allow(dead_code)] pub EntityId);

#[derive(Resource, Default)]
struct OutlineState {
    /// Effective ODF policy, cached by content key for this content database.
    policy: HashMap<ContentKey, bool>,
    content: Option<std::sync::Arc<sta_content::ContentDb>>,
    active: BTreeMap<EntityId, (Entity, Entity)>, // root, outline
    sphere: Option<Handle<Mesh>>,
    material: Option<Handle<PersistentShieldMaterial>>,
}

pub struct PersistentShieldsPlugin;

impl Plugin for PersistentShieldsPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<AssetServer>() {
            embedded_asset!(app, "persistent_shields.wesl");
            app.add_plugins(MaterialPlugin::<PersistentShieldMaterial>::default());
        }
        app.init_resource::<OutlineState>()
            .add_systems(
                Update,
                sync_outlines
                    .after(crate::render::units::sync_units)
                    .in_set(GameSet::Interpolate),
            )
            .add_systems(OnExit(AppState::InGame), clear_outlines);
    }
}

/// Native ParameterDB parsing accepts these words or a nonzero decimal prefix.
/// The ODF resolver has already applied includes and child overrides.
fn enabled(raw: &sta_odf::Odf) -> bool {
    let Some(value) = raw.str("alwaysShowShields") else {
        return false;
    };
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "true" | "yes" | "on") {
        return true;
    }
    let signed = value.strip_prefix(['+', '-']).unwrap_or(&value);
    signed
        .bytes()
        .take_while(u8::is_ascii_digit)
        .any(|d| d != b'0')
}

fn show(unit: &UnitView) -> bool {
    !unit.flags.contains(UnitFlags::CLOAKED)
        && !unit.flags.contains(UnitFlags::DYING)
        && !unit.flags.contains(UnitFlags::BUILDING)
        && unit
            .shields
            .iter()
            .any(|pool| pool[0] > sta_sim::Fixed::ZERO)
}

fn shape(
    bounds: Option<&ModelBounds>,
    radius: Option<&SelectRadius>,
    def_radius: f32,
) -> Transform {
    let b = bounds.copied().unwrap_or_else(|| {
        let radius = radius
            .map_or(def_radius.max(FALLBACK_RADIUS), |r| r.0)
            .max(1.0);
        ModelBounds::fallback(radius)
    });
    let half = ((b.max - b.min) * 0.5).max(Vec3::splat(1.0)) * PAD;
    Transform::from_translation((b.min + b.max) * 0.5).with_scale(half)
}

fn clear_outlines(
    mut commands: Commands,
    mut state: ResMut<OutlineState>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<PersistentShieldMaterial>>>,
) {
    for (_, (_, entity)) in std::mem::take(&mut state.active) {
        commands.entity(entity).try_despawn();
    }
    state.policy.clear();
    state.content = None;
    if let (Some(mut meshes), Some(handle)) = (meshes, state.sphere.take()) {
        meshes.remove(&handle);
    }
    if let (Some(mut materials), Some(handle)) = (materials, state.material.take()) {
        materials.remove(&handle);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_outlines(
    mut commands: Commands,
    app_state: Option<Res<State<AppState>>>,
    flow: Option<Res<crate::game::flow::Flow>>,
    link: Option<Res<SimLink>>,
    local: Option<Res<LocalPlayer>>,
    content: Option<Res<ContentRes>>,
    roots: Option<Res<SimEntities>>,
    root_info: Query<(
        Option<&ModelBounds>,
        Option<&SelectRadius>,
        Option<&Visibility>,
    )>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<PersistentShieldMaterial>>>,
    mut state: ResMut<OutlineState>,
) {
    if flow.is_some() && !app_state.is_some_and(|s| *s.get() == AppState::InGame) {
        clear_outlines(commands, state, meshes, materials);
        return;
    }
    let (Some(link), Some(content), Some(roots)) = (link, content, roots) else {
        clear_outlines(commands, state, meshes, materials);
        return;
    };
    if !state
        .content
        .as_ref()
        .is_some_and(|old| std::sync::Arc::ptr_eq(old, &content.0))
    {
        state.policy.clear();
        state.content = Some(content.0.clone());
    }
    if let (Some(materials), Some(handle), Some(eye)) = (
        materials.as_deref_mut(),
        state.material.as_ref(),
        cameras.iter().next(),
    ) {
        if let Some(mut material) = materials.get_mut(handle) {
            material.params.eye = eye.translation().extend(1.0);
        }
    }

    let visible = match local.as_ref().and_then(|l| l.0) {
        Some(player) => link.curr().visible_to(player),
        None => link.curr().units.iter().collect(),
    };
    let mut desired = BTreeMap::new();
    for unit in visible {
        let Some(&root) = roots.0.get(&unit.id) else {
            continue;
        };
        let Ok((bounds, radius, visibility)) = root_info.get(root) else {
            continue;
        };
        if visibility.is_some_and(|v| *v == Visibility::Hidden) || !show(unit) {
            continue;
        }
        let Some(def) = content.0.unit(unit.def) else {
            continue;
        };
        if !matches!(
            EntityKind::of(&def.class, &def.movement),
            EntityKind::Craft | EntityKind::Station
        ) {
            continue;
        }
        let enabled = *state
            .policy
            .entry(unit.def)
            .or_insert_with(|| enabled(&def.raw));
        if !enabled {
            continue;
        }
        desired.insert(unit.id, (root, shape(bounds, radius, def.radius.to_f32())));
    }
    for (id, (old_root, entity)) in std::mem::take(&mut state.active) {
        match desired.get(&id) {
            Some((root, transform)) if *root == old_root && commands.get_entity(entity).is_ok() => {
                commands.entity(entity).insert(*transform);
                desired.remove(&id);
                state.active.insert(id, (old_root, entity));
            }
            _ => {
                commands.entity(entity).try_despawn();
            }
        }
    }
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    for (id, (root, transform)) in desired {
        let mesh = state
            .sphere
            .get_or_insert_with(|| meshes.add(Sphere::new(1.0)))
            .clone();
        let material = state
            .material
            .get_or_insert_with(|| materials.add(PersistentShieldMaterial { params: default() }))
            .clone();
        let entity = commands
            .spawn((
                PersistentShield(id),
                Mesh3d(mesh),
                MeshMaterial3d(material),
                transform,
                Visibility::Inherited,
                ChildOf(root),
            ))
            .id();
        state.active.insert(id, (root, entity));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::command::{Command, LocalRouter};
    use sta_sim::{Fixed, PlayerId, SimEvent, TickOutput, WorldSource, WorldView};
    use std::sync::Arc;

    struct Frozen(Arc<WorldView>);
    impl WorldSource for Frozen {
        fn view(&self) -> Arc<WorldView> {
            self.0.clone()
        }
        fn step(&mut self, _: &[Command]) -> TickOutput {
            TickOutput {
                view: self.0.clone(),
                events: Vec::new(),
            }
        }
    }

    fn unit(id: u32, key: ContentKey, shields: i32, flags: UnitFlags) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: key,
            owner: PlayerId(0),
            pos: sta_core::Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::from_int(100); 2],
            shields: [[Fixed::from_int(shields), Fixed::from_int(100)]; 4],
            flags,
            progress: None,
            crew: None,
            energy: None,
            autonomy: None,
        }
    }

    fn link(units: Vec<UnitView>, visible: Option<Vec<EntityId>>) -> SimLink {
        let view = WorldView {
            units,
            visibility: visible.map_or_else(Vec::new, |ids| vec![(PlayerId(0), ids)]),
            ..default()
        };
        SimLink::new(
            Box::new(Frozen(Arc::new(view))),
            Box::new(LocalRouter::new()),
            PlayerId(0),
        )
    }

    fn count(app: &mut App) -> usize {
        let mut q = app.world_mut().query::<&PersistentShield>();
        q.iter(app.world()).count()
    }

    fn db_with(value: Option<&str>) -> sta_content::ContentDb {
        let mut def = sta_content::UnitDef::new("test_ship", "craft");
        if let Some(value) = value {
            def.raw = Arc::new(
                sta_odf::resolve("test_ship", &mut |_| {
                    Some(format!("alwaysShowShields = {value}"))
                })
                .unwrap(),
            );
        }
        sta_content::ContentDb::builder().unit(def).build().unwrap()
    }

    #[test]
    fn resolved_policy_honours_default_inheritance_and_zero_override() {
        let base = "alwaysShowShields = yes\n";
        let inherited = sta_odf::resolve("child", &mut |name| match name {
            "child.odf" => Some("#include \"base.odf\"".into()),
            "base.odf" => Some(base.into()),
            _ => None,
        })
        .unwrap();
        assert!(enabled(&inherited));
        let zero = sta_odf::resolve("child", &mut |name| match name {
            "child.odf" => Some("#include \"base.odf\"\nalwaysShowShields = 0".into()),
            "base.odf" => Some(base.into()),
            _ => None,
        })
        .unwrap();
        assert!(!enabled(&zero));
        assert!(!enabled(&sta_odf::Odf::default()));
        for value in ["1", "on", "true", " +12suffix "] {
            let odf = sta_odf::resolve("test", &mut |_| {
                Some(format!("alwaysShowShields = {value}"))
            })
            .unwrap();
            assert!(enabled(&odf), "{value}");
        }
        let invalid =
            sta_odf::resolve("test", &mut |_| Some("alwaysShowShields = --1".into())).unwrap();
        assert!(!enabled(&invalid));
    }

    #[test]
    fn shader_validates() {
        crate::render::fx::validate_wgsl(include_str!("persistent_shields.wesl"));
    }

    #[test]
    fn presentation_rule_does_not_change_content_digest() {
        assert_eq!(db_with(None).hash(), db_with(Some("0")).hash());
        assert_eq!(db_with(None).hash(), db_with(Some("1")).hash());
    }

    #[test]
    fn ship_and_station_lifecycle_is_singleton_and_releases_assets() {
        let mut ship = sta_content::UnitDef::new("ship", "craft");
        ship.raw = Arc::new(
            sta_odf::resolve("ship", &mut |_| Some("alwaysShowShields = 1".into())).unwrap(),
        );
        let mut station = sta_content::UnitDef::new("station", "starbase");
        station.raw = ship.raw.clone();
        let db = Arc::new(
            sta_content::ContentDb::builder()
                .unit(ship)
                .unit(station)
                .build()
                .unwrap(),
        );
        let sk = db.key("ship").unwrap();
        let tk = db.key("station").unwrap();
        let mut app = App::new();
        app.set_error_handler(bevy::ecs::error::panic)
            .add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<PersistentShieldMaterial>>()
            .insert_resource(crate::clock::AnimClock::default())
            .add_message::<crate::sim_link::SimMsg>()
            .insert_resource(ContentRes(db))
            .insert_resource(LocalPlayer(Some(PlayerId(0))))
            .add_plugins((
                PersistentShieldsPlugin,
                crate::render::fx::impacts::ImpactFxPlugin,
            ));
        crate::app::configure_sets(&mut app);
        let a = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                ModelBounds::fallback(20.0),
            ))
            .id();
        let b = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                SelectRadius(30.0),
            ))
            .id();
        app.insert_resource(SimEntities(BTreeMap::from([
            (EntityId(1), a),
            (EntityId(2), b),
        ])));
        let units = |ship_shields, station_shields, flags| {
            vec![
                unit(1, sk, ship_shields, flags),
                unit(2, tk, station_shields, UnitFlags::SELECTABLE),
            ]
        };
        app.insert_resource(link(units(100, 100, UnitFlags::SELECTABLE), None));
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(count(&mut app), 2);
        app.world_mut()
            .write_message(crate::sim_link::SimMsg(SimEvent::Hit {
                target: EntityId(1),
                by: EntityId(2),
                weapon: ContentKey::NONE,
                facing: 0,
                shield: Fixed::from_int(10),
                hull: Fixed::ZERO,
                at: sta_core::Vec3::ZERO,
            }));
        app.update();
        let mut hits = app
            .world_mut()
            .query::<&crate::render::fx::impacts::ImpactFx>();
        assert_eq!(
            hits.iter(app.world())
                .filter(|hit| hit.kind == crate::render::fx::impacts::ImpactKind::Shield)
                .count(),
            1
        );
        assert_eq!(
            count(&mut app),
            2,
            "ordinary impact must leave the persistent child alone"
        );
        assert_eq!(app.world().resource::<Assets<Mesh>>().iter().count(), 1);
        assert_eq!(
            app.world()
                .resource::<Assets<PersistentShieldMaterial>>()
                .iter()
                .count(),
            1
        );
        // A drained pool hides only its own shell; recharge restores it without another material.
        app.insert_resource(link(units(0, 100, UnitFlags::SELECTABLE), None));
        app.update();
        assert_eq!(count(&mut app), 1);
        app.insert_resource(link(units(1, 100, UnitFlags::SELECTABLE), None));
        app.update();
        assert_eq!(count(&mut app), 2);
        // Cloaking, explicit per-player fog and root loss all remove the shell.
        app.insert_resource(link(
            units(1, 100, UnitFlags::SELECTABLE | UnitFlags::CLOAKED),
            None,
        ));
        app.update();
        assert_eq!(count(&mut app), 1);
        app.insert_resource(link(
            units(1, 100, UnitFlags::SELECTABLE),
            Some(vec![EntityId(1)]),
        ));
        app.update();
        assert_eq!(count(&mut app), 1);
        app.world_mut().entity_mut(a).despawn();
        app.update();
        assert_eq!(count(&mut app), 0);
        // A state transition cleans even if another scene installs its own SimLink immediately.
        let c = app
            .world_mut()
            .spawn((Transform::IDENTITY, Visibility::Inherited))
            .id();
        app.world_mut()
            .resource_mut::<SimEntities>()
            .0
            .insert(EntityId(2), c);
        app.insert_resource(link(vec![unit(2, tk, 1, UnitFlags::SELECTABLE)], None));
        app.update();
        assert_eq!(count(&mut app), 1);
        app.world_mut().run_schedule(OnExit(AppState::InGame));
        assert_eq!(count(&mut app), 0);
        assert_eq!(app.world().resource::<Assets<Mesh>>().iter().count(), 0);
        app.init_state::<AppState>();
        app.insert_resource(crate::game::flow::Flow);
        app.update(); // The front end still has a menu SimLink at this point.
        assert_eq!(count(&mut app), 0);
        assert_eq!(app.world().resource::<Assets<Mesh>>().iter().count(), 0);
        app.world_mut().remove_resource::<SimLink>();
        app.update();
        assert_eq!(app.world().resource::<Assets<Mesh>>().iter().count(), 0);
        assert_eq!(
            app.world()
                .resource::<Assets<PersistentShieldMaterial>>()
                .iter()
                .count(),
            0
        );
    }
}

#[cfg(test)]
#[path = "persistent_shields/gpu_tests.rs"]
mod gpu_tests;
