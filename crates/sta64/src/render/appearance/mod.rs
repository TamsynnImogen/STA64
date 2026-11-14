//! Native, per-instance faction and subsystem presentation (RND-33).

pub mod effects;
pub mod factions;
pub mod policy;
mod selection;
pub mod subsystems;

use super::{
    model_scene::{NativeNode, TeamPart},
    textures::LoaderParams,
    units::{HullShare, TeamColour},
};
use crate::{
    app::GameSet,
    sim_link::{SimEntity, SimLink},
    viewer::Mounted,
};
use bevy::prelude::*;
use policy::{AppearancePolicy, SubsystemKind};
use sta_assets::model::Model;
use sta_core::ContentKey;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use subsystems::{SystemVisualState, VisualPlan};

/// The immutable source model is retained for projection and exact subtree IDs.
#[derive(Component)]
pub struct ModelInstance {
    pub def: ContentKey,
    pub model: Arc<Model>,
    pub scale: f32,
    pub policy: Arc<AppearancePolicy>,
}

impl ModelInstance {
    pub fn new(
        def: ContentKey,
        model: Arc<Model>,
        scale: f32,
        unit: &sta_content::UnitDef,
    ) -> Self {
        let (policy, warnings) = policy::parse_appearance_policy(&unit.raw);
        for message in warnings {
            warn!("{}: {message}", unit.odf);
        }
        Self {
            def,
            model,
            scale,
            policy: Arc::new(policy),
        }
    }
}

#[derive(Component, Clone, Copy)]
pub struct ModelNode {
    pub root: Entity,
    pub index: usize,
}

/// The shared source is immutable. Appearance clones it once for this part.
#[derive(Component)]
pub struct ModelPart {
    pub node: usize,
    pub group: usize,
    pub base: Handle<StandardMaterial>,
}

#[derive(Component)]
pub struct PrivateMaterial {
    pub handle: Handle<StandardMaterial>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppearanceSet;

pub struct AppearancePlugin;
impl Plugin for AppearancePlugin {
    fn build(&self, app: &mut App) {
        effects::install(app);
        app.add_systems(
            Update,
            sync.in_set(AppearanceSet)
                .after(GameSet::Interpolate)
                .before(super::hulls::HullApply)
                .run_if(crate::game::flow::match_running)
                .run_if(resource_exists::<SimLink>)
                .run_if(resource_exists::<Mounted>),
        );
    }
}

#[derive(Default)]
struct InstanceState {
    systems: [SystemVisualState; 5],
    signature: Option<(String, Option<String>, [u8; 4])>,
    selected: [Option<usize>; 5],
}

fn system_index(kind: SubsystemKind) -> usize {
    match kind {
        SubsystemKind::Engines => 0,
        SubsystemKind::Weapons => 1,
        SubsystemKind::Shields => 2,
        SubsystemKind::LifeSupport => 3,
        SubsystemKind::Sensors => 4,
    }
}

/// The node and every descendant are tested by index, never by ambiguous names.
pub fn within(model: &Model, mut child: usize, ancestor: usize) -> bool {
    for _ in 0..=model.nodes.len() {
        if child == ancestor {
            return true;
        }
        let Some(parent) = model.nodes.get(child).and_then(|n| n.parent) else {
            return false;
        };
        child = parent;
    }
    false
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync(
    mut commands: Commands,
    link: Res<SimLink>,
    mounted: Res<Mounted>,
    roots: Query<(Entity, &SimEntity, &ModelInstance, &HullShare, &TeamColour)>,
    mut nodes: Query<(Entity, &ModelNode, &Name, &mut Visibility, Has<NativeNode>)>,
    mut parts: Query<
        (
            Entity,
            &ModelPart,
            &ChildOf,
            &mut Transform,
            Option<&PrivateMaterial>,
            Has<TeamPart>,
        ),
        Without<ModelNode>,
    >,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut textures: LoaderParams,
    mut catalogue: Local<Option<factions::FactionCatalogue>>,
    mut state: Local<HashMap<Entity, InstanceState>>,
    mut effects: MessageWriter<effects::PartEffect>,
    construction_decor: Query<&super::construction::ConstructionDecor>,
    fresh: Query<(), Or<(Added<ModelInstance>, Added<ModelPart>, Changed<TeamColour>)>>,
    mut seen: Local<Option<Arc<sta_sim::WorldView>>>,
    mut flipbooks: Query<&mut super::sprites::UvFlipbook>,
) {
    if !mounted.is_changed()
        && fresh.is_empty()
        && seen
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, link.curr()))
    {
        return;
    }
    *seen = Some(link.curr().clone());
    if catalogue.is_none() || mounted.is_changed() {
        *catalogue = Some(factions::FactionCatalogue::load(&mounted.0));
        state.clear();
    }
    let catalogue = catalogue.as_ref().expect("loaded");
    let live: HashSet<_> = roots.iter().map(|r| r.0).collect();
    state.retain(|root, _| live.contains(root));
    let mut node_groups: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (entity, node, _, _, _) in &nodes {
        node_groups.entry(node.root).or_default().push(entity);
    }
    let mut part_groups: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (entity, _, parent, _, _, _) in &parts {
        if let Ok((_, node, _, _, _)) = nodes.get(parent.parent()) {
            part_groups.entry(node.root).or_default().push(entity);
        }
    }
    let view = link.curr();
    for (root, sim, instance, share, team) in &roots {
        let Some(unit) = view.unit(sim.0) else {
            continue;
        };
        let selected_faction = selection::faction_identifier(view, unit)
            .and_then(|f| catalogue.get(f));
        let faction_name = selected_faction.map(|f| f.name.as_str());
        let suffix = selected_faction.and_then(|f| f.suffix.as_deref());
        let has_nodes = instance
            .model
            .nodes
            .iter()
            .any(|n| catalogue.is_faction_node(&n.name));
        if suffix.is_none()
            && !has_nodes
            && instance.policy.subsystem_nodes.is_empty()
            && !state.contains_key(&root)
        {
            continue;
        }
        let rgba = team.0.to_srgba().to_u8_array();
        let signature = (
            faction_name.unwrap_or("").to_owned(),
            suffix.map(str::to_owned),
            rgba,
        );
        let state = state.entry(root).or_default();
        let material_changed = state.signature.as_ref() != Some(&signature);
        state.signature = Some(signature);
        let node_ids = node_groups
            .get(&root)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let live_nodes: HashSet<usize> = node_ids
            .iter()
            .filter_map(|e| nodes.get(*e).ok().map(|(_, n, _, _, _)| n.index))
            .collect();
        let candidates: [Vec<_>; 5] = std::array::from_fn(|system| {
            instance
                .policy
                .subsystem_nodes
                .iter()
                .filter(|spec| system_index(spec.kind) == system)
                .flat_map(|spec| {
                    instance
                        .model
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(i, n)| {
                            live_nodes.contains(i) && n.name.eq_ignore_ascii_case(&spec.node)
                        })
                        .map(move |(i, _)| (i, spec))
                })
                .collect()
        });
        let plans: [VisualPlan; 5] = std::array::from_fn(|i| {
            let health = view
                .appearance
                .systems(sim.0)
                .and_then(|s| s[i])
                .map(|s| s.health.map(|v| v.to_f32()));
            state.systems[i].update(sim.0, i, health, candidates[i].len())
        });
        for (i, plan) in plans.iter().enumerate() {
            let selected = plan.selected.or(state.selected[i]);
            if let Some((node, spec)) = selected.and_then(|n| candidates[i].get(n)) {
                let kind = if plan.just_destroyed {
                    Some(effects::PartEffectKind::Destroyed {
                        explosion: spec.explosion.clone(),
                    })
                } else if plan.just_completed {
                    Some(effects::PartEffectKind::Completed)
                } else if plan.repair_progress_delta > 0.0 {
                    Some(effects::PartEffectKind::Repair)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    effects.write(effects::PartEffect {
                        root,
                        node: *node,
                        kind,
                        seed: sim
                            .0
                            .0
                            .wrapping_add(view.tick as u32)
                            .wrapping_add(i as u32),
                    });
                }
            }
            state.selected[i] = plan.selected;
        }
        for entity in node_ids {
            let Ok((_, node, name, mut visibility, native)) = nodes.get_mut(*entity) else {
                continue;
            };
            let construction_hidden = construction_decor.contains(*entity)
                && unit.progress.is_some_and(|p| p < sta_core::Fixed::ONE);
            let ordinary = !construction_hidden
                && (native
                    || super::units::damage_threshold(name).is_none_or(|limit| share.0 < limit));
            let faction = !catalogue.is_faction_node(name)
                || faction_name.is_some_and(|f| f.eq_ignore_ascii_case(name));
            let mut visible = ordinary && faction;
            for (i, plan) in plans.iter().enumerate() {
                if let Some((selected, _)) = plan.selected.and_then(|n| candidates[i].get(n))
                    && within(&instance.model, node.index, *selected)
                    && (node.index != *selected || plan.root_scale <= 0.0)
                {
                    visible = false;
                }
            }
            *visibility = if visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        for entity in part_groups
            .get(&root)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Ok((entity, part, parent, mut transform, private, is_team)) =
                parts.get_mut(*entity)
            else {
                continue;
            };
            let Ok((_, node, _, _, _)) = nodes.get(parent.parent()) else {
                continue;
            };
            if node.root != root {
                continue;
            }
            let mut scale = 1.0_f32;
            for (i, plan) in plans.iter().enumerate() {
                if plan
                    .selected
                    .and_then(|n| candidates[i].get(n))
                    .is_some_and(|(selected, _)| part.node == *selected)
                {
                    scale = scale.min(plan.root_scale);
                }
            }
            transform.scale = Vec3::splat(scale);
            // Plain materials stay shared until this instance actually needs a
            // variant (or its team colour changes on capture).
            if !material_changed && private.is_some() {
                continue;
            }
            if suffix.is_none() && private.is_none() && !is_team {
                continue;
            }
            let Some(mut material) = standard.get(&part.base).cloned() else {
                continue;
            };
            if is_team {
                material.base_color = team.0;
            }
            let group = instance
                .model
                .nodes
                .get(part.node)
                .and_then(|n| match &n.kind {
                    sta_assets::model::NodeKind::Mesh(mesh) => mesh.groups.get(part.group),
                    _ => None,
                });
            if let (Some(suffix), Some(group)) = (suffix, group) {
                let mut loader = textures.loader();
                if let Some(name) = &group.maps.diffuse
                    && let Some(texture) = loader.faction_colour(name, suffix)
                {
                    material.base_color_texture = Some(texture);
                }
                // Native faction bump maps are not supported: retain slot 1.
                if let Some(name) = &group.maps.glow
                    && let Some(texture) = loader.faction_colour(name, suffix)
                {
                    material.emissive_texture = Some(texture);
                    material.emissive = LinearRgba::WHITE;
                }
                if let Some(name) = &group.maps.specular
                    && let Some(texture) = loader.faction_roughness(name, suffix)
                {
                    material.metallic_roughness_texture = Some(texture);
                    material.perceptual_roughness = 1.0;
                }
            }
            if let Some(private) = private {
                if let Some(mut target) = standard.get_mut(&private.handle) {
                    if flipbooks.contains(entity) {
                        material.uv_transform = target.uv_transform;
                    }
                    *target = material;
                    // Capture can introduce or remove glow/specular maps. Recheck
                    // hull-shader eligibility in HullApply this same frame rather
                    // than leave a previously converted part on the old pipeline.
                    commands
                        .entity(entity)
                        .insert(MeshMaterial3d(private.handle.clone()))
                        .remove::<super::hulls::HullSource>()
                        .remove::<MeshMaterial3d<super::hulls::HullMaterial>>();
                }
            } else {
                let handle = standard.add(material);
                if let Ok(mut flipbook) = flipbooks.get_mut(entity) {
                    flipbook.material = handle.clone();
                }
                commands
                    .entity(entity)
                    .insert((
                        PrivateMaterial {
                            handle: handle.clone(),
                        },
                        MeshMaterial3d(handle),
                    ))
                    .remove::<super::hulls::HullSource>()
                    .remove::<MeshMaterial3d<super::hulls::HullMaterial>>();
            }
        }
    }
}

#[cfg(test)]
mod tests;
