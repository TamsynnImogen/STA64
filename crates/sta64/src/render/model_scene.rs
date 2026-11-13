//! Turn a loaded model (SOD or M3D, see `sta_assets::model`) into a Bevy entity
//! hierarchy.
//!
//! Coordinate conversion: the files are Direct3D left-handed with row-vector matrices;
//! Bevy is right-handed. Positions and transforms are mirrored on Z
//! (`M' = S M S`, `S = diag(1, 1, -1)`). Triangle winding is kept as stored:
//! with the mirror applied, SOD order is front-facing in Bevy (the M3D loader
//! already stores its faces in SOD order).

use crate::render::sprites::{self, Billboard, SpriteFx, UvFlipbook};
use crate::render::textures::Loader;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::{Face, PrimitiveTopology};
use sta_assets::a2fmt::Matrix34;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_assets::model::{Group, Keys, Mesh as ModelMesh, Model, NodeKind};
use sta_assets::spr_anim;
use std::collections::{HashMap, HashSet};

mod pbr;

#[cfg(test)]
#[path = "model_scene/gltf_tests.rs"]
mod gltf_tests;

#[cfg(test)]
#[path = "model_scene/defiant_tests.rs"]
mod defiant_tests;

#[cfg(test)]
#[path = "model_scene/defiant_gpu.rs"]
mod defiant_gpu;

/// Step-keyed SOD animation on one node. Keys are evenly spaced over `period`.
#[derive(Component)]
pub struct NodeAnim {
    pub period: f32,
    pub base: Transform,
    pub keys: AnimKeys,
}

pub enum AnimKeys {
    Matrix(Vec<Transform>),
    Scale(Vec<f32>),
}

/// Built meshes and materials shared by every copy of a model (maps place
/// hundreds of the same nebula or ship). Key: model, node, group, team colour,
/// Borg and normal-map switches.
#[derive(Resource, Default)]
pub struct PartCache {
    map: HashMap<PartKey, CachedPart>,
}

/// Model, node, group, team colour, Borg, normal maps, unlit, ScaleSOD bits,
/// and whether the shared source must remain CPU-readable for construction.
type PartKey = (String, usize, usize, [u8; 3], bool, bool, bool, u32, bool);

#[derive(Clone)]
struct CachedPart {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    team: bool,
    bumped: bool,
    triangles: usize,
    /// Set when the part's texture names a surface recipe (rocks, moons).
    rock: Option<crate::render::rock::RockPart>,
    /// Set on a hull's parts (not skies), for the hull look (`render/hulls.rs`).
    hull: Option<crate::render::hulls::HullPart>,
}

/// Face groups that take the owner's team colour.
#[derive(Component)]
pub struct TeamPart;

/// Native static scene names do not opt into legacy damage-node conventions.
#[derive(Component)]
pub struct NativeNode;

#[derive(Default)]
pub struct SpawnStats {
    pub meshes: usize,
    pub triangles: usize,
    pub missing_textures: Vec<String>,
    pub animated_nodes: usize,
    pub bump_meshes: usize,
    pub sprite_nodes: usize,
    pub unknown_sprite_nodes: Vec<String>,
    pub emitters: usize,
    pub flipbooks: usize,
    /// Model-space bounds (Bevy space): centre and radius.
    pub centre: Vec3,
    pub radius: f32,
}

pub fn to_bevy(m: &Matrix34) -> Transform {
    let a = |i: usize| {
        let v = m.axis(i);
        Vec4::new(v[0], v[1], v[2], 0.0)
    };
    let t = m.translation();
    let mat = Mat4::from_cols(a(0), a(1), a(2), Vec4::new(t[0], t[1], t[2], 1.0));
    let s = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    Transform::from_matrix(s * mat * s)
}

fn mirror(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[1], -p[2]]
}

/// Alpha mode and whether the group is unlit, from its blend mode.
fn blend(group: &Group) -> (AlphaMode, bool) {
    let m = crate::render::materials::lookup(&group.blend);
    (m.alpha_mode, m.unlit)
}

pub struct SpawnCtx<'a, 'w, 's> {
    pub commands: &'a mut Commands<'w, 's>,
    pub meshes: &'a mut Assets<Mesh>,
    pub materials: &'a mut Assets<StandardMaterial>,
    pub textures: Loader<'a>,
    pub team: Color,
    /// Use Borg-assimilated `_b` texture variants where they exist.
    pub borg: bool,
    pub sprites: &'a SpriteTable,
    /// Show damage-state nodes (breaches, fires, venting) as on a crippled craft.
    pub damaged: bool,
    /// Use normal maps (N in the viewer, for comparing).
    pub normal_maps: bool,
    /// Share built parts between copies of the same model (`model_key`).
    pub parts: Option<&'a mut PartCache>,
    pub model_key: String,
    /// Draw everything unlit (skyboxes).
    pub unlit: bool,
    /// The ODF's `ScaleSOD`. As in the engine it scales the model's positions
    /// (node offsets and vertices) but not sprite sizes or particles.
    pub scale: f32,
}

/// Spawn `model` under `root`. Only the finest LOD level of each LOD switch is shown.
pub fn spawn(ctx: &mut SpawnCtx, model: &Model, root: Entity) -> SpawnStats {
    spawn_impl(ctx, model, root, None)
}

/// Spawn an unfinished instance with deterministic spatial geometry reveal.
/// Construction data is attached only to partial instances; progress is applied
/// before the first frame so no fully-built hull is briefly visible.
pub fn spawn_construction(
    ctx: &mut SpawnCtx,
    model: &Model,
    root: Entity,
    chunks: u16,
    progress: f32,
) -> SpawnStats {
    let progress = progress.clamp(0.0, 1.0);
    if progress >= 1.0 {
        spawn(ctx, model, root)
    } else {
        spawn_impl(ctx, model, root, Some((chunks.clamp(1, 256), progress)))
    }
}

fn spawn_impl(
    ctx: &mut SpawnCtx,
    model: &Model,
    root: Entity,
    construction: Option<(u16, f32)>,
) -> SpawnStats {
    let mut stats = SpawnStats::default();

    // Hide every LOD level except the most detailed.
    let legacy = model.format != sta_assets::model::SourceFormat::Gltf;
    let mut hidden: HashSet<usize> = if legacy { model.coarse_lods().into_iter().collect() } else { HashSet::new() };
    if legacy && !ctx.damaged {
        hidden.extend(model.damage_nodes());
    }
    if legacy && !ctx.borg {
        hidden.extend(model.borg_node());
    }
    let mut visible_nodes = HashSet::new();
    let mut visible_stack: Vec<usize> = model.roots().collect();
    while let Some(index) = visible_stack.pop() {
        if hidden.contains(&index) {
            continue;
        }
        visible_nodes.insert(index);
        visible_stack.extend(model.nodes[index].children.iter().copied());
    }
    // Unit attachment keeps damage geometry in the tree for show_damage. It
    // remains opaque/hidden through construction and is not part of the reveal.
    let mut damage_stack = if legacy {
        model.damage_nodes()
    } else {
        Vec::new()
    };
    while let Some(index) = damage_stack.pop() {
        visible_nodes.remove(&index);
        damage_stack.extend(model.nodes[index].children.iter().copied());
    }
    let plan = construction.and_then(|(chunks, _)| {
        crate::render::construction::ConstructionPlan::build(
            model,
            chunks,
            ctx.scale,
            &visible_nodes,
        )
    });
    let completed = match (construction, plan.as_ref()) {
        (Some((_, progress)), Some(plan)) => (progress * plan.chunk_count as f32).floor() as usize,
        _ => 0,
    };

    let anims: HashMap<String, &sta_assets::model::Channel> =
        model.channels.iter().map(|c| (c.node.to_ascii_lowercase(), c)).collect();
    // Texture flipbooks on a mesh's UV offset.
    let flipbooks: HashMap<String, (String, f32)> = model
        .flipbooks
        .iter()
        .map(|(node, anim, offset)| (node.to_ascii_lowercase(), (anim.clone(), *offset)))
        .collect();
    let quad = ctx.meshes.add(sprites::unit_quad());

    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);

    let mut entities: Vec<Option<Entity>> = vec![None; model.nodes.len()];
    // Parents precede children only by chance in files; walk the tree from roots.
    let mut stack: Vec<usize> = model.roots().collect::<Vec<_>>();
    stack.reverse();
    while let Some(i) = stack.pop() {
        let node = &model.nodes[i];
        if hidden.contains(&i) {
            continue;
        }
        let parent = node.parent.and_then(|p| entities[p]).unwrap_or(root);
        let mut base = to_bevy(&node.transform);
        base.translation *= ctx.scale;
        let mut e = ctx.commands.spawn((Name::new(node.name.clone()), base, Visibility::default(), ChildOf(parent),
        ));
        e.insert(crate::render::appearance::ModelNode { root, index: i });
        if !legacy { e.insert(NativeNode); }

        let key_count = anims.get(&node.name.to_ascii_lowercase()).map_or(0, |c| match &c.keys {
            Keys::Matrix(k) => k.len(),
            Keys::Scale(k) => k.len(),
        });
        if let Some(ch) = anims.get(&node.name.to_ascii_lowercase())
            && key_count > 1
            && ch.period > 0.0
        {
            let keys = match &ch.keys {
                Keys::Matrix(k) => AnimKeys::Matrix(
                    k.iter()
                        .map(|m| {
                            let mut t = to_bevy(m);
                            t.translation *= ctx.scale;
                            t
                        })
                        .collect(),
                ),
                Keys::Scale(k) => AnimKeys::Scale(k.clone()),
            };
            let track = NodeAnim {
                period: ch.period,
                base,
                keys,
            };
            // Open-ended tracks (bay doors, build arms) wait for their event.
            if crate::render::anim::classify(&ch.keys) == crate::render::anim::TrackKind::Event {
                e.insert(crate::render::anim::EventAnim::new(track, root));
            } else {
                e.insert(track);
            }
            stats.animated_nodes += 1;
        }
        let id = e.id();
        entities[i] = Some(id);

        if let NodeKind::Mesh(mesh) = &node.kind {
            let world = model.model_transform(i);
            for p in &mesh.positions {
                let w = Vec3::from(mirror(world.transform_point(*p))) * ctx.scale;
                min = min.min(w);
                max = max.max(w);
            }
            let flip = flipbooks.get(&node.name.to_ascii_lowercase());
            spawn_mesh(ctx, model, i, mesh, id, flip,
                plan.as_ref().map(|plan| (plan, root, completed)),
                &mut stats,
            );
        }
        match &node.kind {
            NodeKind::Sprite => spawn_sprite_node(ctx, node.sprite_node_id(), id, &quad,
                construction.map(|_| root),
                &mut stats,
            ),
            NodeKind::Emitter(emitter) => {
                match crate::render::particles::Template::new(ctx.sprites, emitter) {
                Some(tpl) => {
                    ctx.commands.entity(id).insert(crate::render::particles::Emitter {
                        tpl: std::sync::Arc::new(tpl),
                        owner: root,
                        timer: 0.0,
                        rng: Default::default(),
                    });
                        if construction.is_some() {
                            ctx.commands.entity(id).insert((
                                crate::render::construction::ConstructionDecor { owner: root },
                                Visibility::Hidden,
                            ));
                        }
                    stats.emitters += 1;
                }
                None => stats.unknown_sprite_nodes.push(format!("emitter {emitter}")),
            }
            }
            _ => {}
        }

        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }

    if min.x <= max.x {
        stats.centre = (min + max) * 0.5;
        stats.radius = ((max - min).length() * 0.5).max(0.01);
    } else {
        stats.radius = 1.0;
    }
    stats.missing_textures.sort();
    stats.missing_textures.dedup();
    stats.unknown_sprite_nodes.sort();
    stats.unknown_sprite_nodes.dedup();
    stats
}

fn spawn_sprite_node(ctx: &mut SpawnCtx, id: &str, parent: Entity, quad: &Handle<Mesh>,
    construction_owner: Option<Entity>,
    stats: &mut SpawnStats,
) {
    let table = ctx.sprites;
    // The moons' round nimbus: their cracks carry the light instead (RND-05).
    if id.eq_ignore_ascii_case("oreglow") && !crate::render::rock::MOON_NIMBUS {
        return;
    }
    let Some(def) = table.sprite_node(id) else {
        stats.unknown_sprite_nodes.push(id.to_owned());
        return;
    };
    let Some(sprite) = table.sprite(&def.sprite) else {
        stats.unknown_sprite_nodes.push(format!("{id} (sprite {})", def.sprite));
        return;
    };
    let texture = ctx.textures.colour(&sprite.texture, false);
    if texture.is_none() {
        stats.missing_textures.push(sprite.texture.clone());
    }
    let material = ctx.materials.add(StandardMaterial {
        base_color: Color::srgb(def.colour[0], def.colour[1], def.colour[2]),
        base_color_texture: texture,
        alpha_mode: sprites::alpha_mode(spr_anim::sprite_blend(&sprite.material)),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        uv_transform: sprites::uv_affine(spr_anim::sprite_uv(table, sprite, 0.0)),
        ..default()
    });
    // Billboards are sized by half extents, flat sprites by full size.
    let (scale, billboard) = if def.billboard {
        (Vec3::new(def.size[0] * 2.0, def.size[1] * 2.0, 1.0), true)
    } else {
        (Vec3::new(def.size[0], def.size[1], 1.0), false)
    };
    let mut e = ctx.commands.spawn((
        Mesh3d(quad.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_scale(scale),
        Visibility::default(),
        SpriteFx {
            node_anim: def.animation.clone(),
            sprite: def.sprite.clone(),
            colour: def.colour,
            material,
        },
        ChildOf(parent),
    ));
    if billboard {
        e.insert(Billboard);
    }
    if let Some(owner) = construction_owner {
        e.insert((
            crate::render::construction::ConstructionDecor { owner },
            Visibility::Hidden,
        ));
    }
    stats.sprite_nodes += 1;
}

#[allow(clippy::too_many_arguments)]
fn spawn_mesh(
    ctx: &mut SpawnCtx,
    model: &Model,
    node_index: usize,
    mesh: &ModelMesh,
    parent: Entity,
    flipbook: Option<&(String, f32)>,
    construction: Option<(
        &crate::render::construction::ConstructionPlan,
        Entity,
        usize,
    )>,
    stats: &mut SpawnStats,
) {
    let two_sided = mesh.two_sided;
    let mut bumped = false;

    let team_rgb = {
        let c = ctx.team.to_srgba();
        [c.red, c.green, c.blue].map(|v| (v * 255.0) as u8)
    };
    for (group_index, group) in mesh.groups.iter().enumerate() {
        if group.faces.is_empty() {
            continue;
        }
        let native = model.pbr.as_ref().and_then(|p| p.materials.get(&group.material).map(|m| (p, m)));
        let attributes = model.pbr.as_ref().and_then(|p| p.attributes.get(&node_index));
        let cache_key = (
            ctx.model_key.clone(),
            node_index,
            group_index,
            team_rgb,
            ctx.borg,
            ctx.normal_maps,
            ctx.unlit,
            ctx.scale.to_bits(),
            construction.is_some(),
        );
        let reveal = construction
            .map(|(plan, _, _)| plan.part_source(model, node_index, group_index, mesh, ctx.scale));
        if let Some(part) = ctx.parts.as_ref().and_then(|c| c.map.get(&cache_key)).cloned() {
            spawn_part(ctx, &part, parent, (node_index, group_index), flipbook,
                construction.map(|(_, root, stage)| (root, reveal.clone().unwrap(), stage)),
                stats,
            );
            bumped |= part.bumped;
            continue;
        }
        let maps = &group.maps;
        let want = |name: &Option<String>, h: Option<Handle<Image>>, stats: &mut SpawnStats| {
            if h.is_none()
                && let Some(n) = name
            {
                stats.missing_textures.push(n.clone());
            }
            h
        };
        let diffuse = maps.diffuse.as_deref().and_then(|n| ctx.textures.colour(n, ctx.borg));
        let texture = want(&maps.diffuse, diffuse, stats);
        // SOD bump path: height map. M3D: bump map, normal or height by content.
        let normal = match (&maps.height, &maps.bump) {
            _ if !ctx.normal_maps => None,
            (Some(h), _) => want(&maps.height, ctx.textures.normal_from_height(h), stats),
            (None, Some(b)) => want(&maps.bump, ctx.textures.normal_from_bump(b), stats),
            _ => None,
        };
        let spec = maps.specular.as_deref().and_then(|n| ctx.textures.roughness_from_specular(n));
        let spec = want(&maps.specular, spec, stats);
        let glow = maps.glow.as_deref().and_then(|n| ctx.textures.colour(n, false));
        let glow = want(&maps.glow, glow, stats);
        let (alpha_mode, additive) = blend(group);
        let mspec = crate::render::materials::lookup(&group.blend);
        let two_sided = two_sided || mspec.double_sided;

        // De-index this group: one vertex per distinct (position, uv) corner.
        let mut remap: HashMap<(u32, u32), u32> = HashMap::new();
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut indices: Vec<u32> = Vec::with_capacity(group.faces.len() * 3);
        let mut source_positions: Vec<usize> = Vec::new();
        for face in &group.faces {
            // Corners are in SOD winding: after the Z mirror that is already
            // counter-clockwise (front-facing) for Bevy. Reversing it turned
            // FO4 hulls inside out in the viewer.
            for &(p, u) in face {
                let (Some(pos), Some(uv)) = (mesh.positions.get(p as usize), mesh.uvs.get(u as usize)) else {
                    continue;
                };
                let idx = *remap.entry((p, u)).or_insert_with(|| {
                    positions.push(mirror(pos.map(|v| v * ctx.scale)));
                    uvs.push(*uv);
                    source_positions.push(p as usize);
                    (positions.len() - 1) as u32
                });
                indices.push(idx);
            }
        }
        indices.truncate(indices.len() / 3 * 3);
        if indices.is_empty() {
            continue;
        }
        // Area-weighted normals by shared position (SOD stores none).
        let mut by_pos: HashMap<[u32; 3], Vec3> = HashMap::new();
        let key = |p: [f32; 3]| p.map(f32::to_bits);
        for t in indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(positions[i as usize]));
            let n = (b - a).cross(c - a);
            for i in t {
                *by_pos.entry(key(positions[*i as usize])).or_default() += n;
            }
        }
        let normals: Vec<[f32; 3]> = positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                attributes.and_then(|a| a.normals.as_ref())
                .and_then(|n| n.get(source_positions[i])).copied().map(mirror)
                .unwrap_or_else(|| by_pos[&key(*p)].normalize_or(Vec3::Y).to_array())
            })
            .collect();

        let triangles = indices.len() / 3;

        let usage = if construction.is_some() {
            RenderAssetUsages::default()
        } else {
            RenderAssetUsages::RENDER_WORLD
        };
        let mut bevy_mesh = Mesh::new(PrimitiveTopology::TriangleList, usage)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(indices));
        if let Some(attributes) = attributes {
            if let Some(values) = &attributes.tangents {
                let values: Option<Vec<[f32; 4]>> = source_positions.iter().map(|&i| values.get(i).map(|v| [v[0],v[1],-v[2],-v[3]])).collect();
                if let Some(values) = values { bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, values); }
            }
            if let Some(values) = &attributes.colors {
                let values: Option<Vec<[f32; 4]>> = source_positions.iter().map(|&i| values.get(i).copied()).collect();
                if let Some(values) = values { bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, values); }
            }
            if let Some(values) = &attributes.uv1 {
                let values: Option<Vec<[f32; 2]>> = source_positions.iter().map(|&i| values.get(i).copied()).collect();
                if let Some(values) = values { bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, values); }
            }
        }
        let normal = normal.filter(|_| bevy_mesh.generate_tangents().is_ok());
        let mut normal_present = normal.is_some();
        bumped |= normal_present;

        let lm = model.lighting_material(&group.material);
        let team = native.is_none() && crate::render::hulls::is_team_material(&group.material);
        let self_illum = lm.is_some_and(|m| m.self_illuminating);
        let base_color = if team { ctx.team } else { Color::WHITE };
        let mut material = if let Some((pbr_model, source)) = native {
            pbr::material(&mut ctx.textures, pbr_model, source, ctx.normal_maps, ctx.unlit, stats,
            )
        } else { StandardMaterial {
            base_color,
            base_color_texture: texture,
            normal_map_texture: normal,
            perceptual_roughness: if spec.is_some() { 1.0 } else { 0.6 },
            metallic_roughness_texture: spec,
            emissive: if glow.is_some() { LinearRgba::WHITE } else { LinearRgba::BLACK },
            emissive_texture: glow,
            alpha_mode,
            unlit: additive || self_illum || ctx.unlit,
            double_sided: two_sided,
            cull_mode: if two_sided { None } else { Some(Face::Back) },
            reflectance: mspec.reflectance,
            ..default()
        }};
        if native.is_some() && material.normal_map_texture.is_some() && bevy_mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_none() {
            // A glTF normal texture can select UV1. Tangent generation uses UV0;
            // temporarily use the selected coordinates without changing the asset.
            let uv0 = if material.normal_map_channel == bevy::mesh::UvChannel::Uv1 {
                match (bevy_mesh.attribute(Mesh::ATTRIBUTE_UV_0).cloned(), bevy_mesh.attribute(Mesh::ATTRIBUTE_UV_1).cloned(),
                ) {
                    (Some(uv0), Some(uv1)) => { bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv1); Some(uv0) }
                    _ => None,
                }
            } else { None };
            let generated = bevy_mesh.generate_tangents();
            if let Some(uv0) = uv0 { bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0); }
            if generated.is_err() {
                warn!("{}: unable to generate native normal-map tangents for node {node_index}", ctx.model_key);
                material.normal_map_texture = None;
            }
        }
        normal_present |= material.normal_map_texture.is_some();
        bumped |= normal_present;
        // A dilithium moon's glow shell is drawn dark (RND-05).
        let material = if crate::render::rock::is_moon_shell(&group.material) { crate::render::rock::shell_material(two_sided) } else { material };
        // A texture with a surface recipe: the part becomes a rock once baked, its
        // surface looked up about the mesh's centre over its extents.
        let rock = maps.diffuse.as_deref().filter(|n| crate::render::surfaces::recipe_text(ctx.textures.vfs, n).is_some()).map(|n| {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &mesh.positions {
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            let c: [f32; 3] = std::array::from_fn(|a| (lo[a] + hi[a]) * 0.5);
            let ext: [f32; 3] = std::array::from_fn(|a| ((hi[a] - lo[a]) * 0.5 * ctx.scale).max(1e-4));
            crate::render::rock::RockPart {
                recipe: n.to_owned(),
                centre: Vec3::from(mirror(c)) * ctx.scale,
                inv_extent: Vec3::from(ext.map(|e| 1.0 / e)),
            }
        });
        // A `default` mesh's texture alpha is a cut-out where its group is not
        // self-illuminating (open girders), as Refit's hulls draw it.
        let hull = (native.is_none() && !ctx.unlit && rock.is_none()).then_some(crate::render::hulls::HullPart {
            self_illum,
            cut_alpha: group.blend == "default" && !self_illum,
        },
        );
        let part = CachedPart {
            mesh: ctx.meshes.add(bevy_mesh),
            material: ctx.materials.add(material),
            team,
            bumped: normal_present,
            triangles,
            rock,
            hull,
        };
        if let Some(cache) = ctx.parts.as_mut() {
            cache.map.insert(cache_key, part.clone());
        }
        spawn_part(ctx, &part, parent, (node_index, group_index), flipbook,
            construction.map(|(_, root, stage)| (root, reveal.unwrap(), stage)),
            stats,
        );
    }
    if bumped {
        stats.bump_meshes += 1;
    }
}

fn spawn_part(ctx: &mut SpawnCtx, part: &CachedPart, parent: Entity, surface: (usize, usize), flipbook: Option<&(String, f32)>,
    construction: Option<(
        Entity,
        std::sync::Arc<crate::render::construction::ConstructionSource>,
        usize,
    )>,
    stats: &mut SpawnStats,
) {
    let mut private = None;
    let mesh = if let Some((root, source, completed)) = construction {
        if let Some(mut partial) = ctx.meshes.get(&part.mesh).cloned() {
            let indices = source
                .stage_indices
                .iter()
                .take(completed)
                .flat_map(|stage| stage.iter().copied())
                .collect();
            partial.insert_indices(Indices::U32(indices));
            partial.asset_usage = RenderAssetUsages::default();
            let handle = ctx.meshes.add(partial);
            private = Some((root, source, completed, handle.clone()));
            handle
        } else {
            warn!("construction mesh source unavailable; withholding full mesh");
            return;
        }
    } else {
        part.mesh.clone()
    };
    stats.triangles += part.triangles;
    stats.meshes += 1;
    let mut e = ctx.commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(part.material.clone()),
        crate::render::appearance::ModelPart { node: surface.0, group: surface.1, base: part.material.clone() },
        Transform::IDENTITY,
        ChildOf(parent),
    ));
    if let Some((owner, source, completed, partial)) = private {
        e.insert(crate::render::construction::ConstructionPart {
            owner,
            original: part.mesh.clone(),
            partial,
            source,
            completed,
        });
    }
    if part.team {
        e.insert(TeamPart);
    }
    if let Some(r) = &part.rock {
        e.insert(r.clone());
    }
    if let Some(h) = part.hull {
        e.insert(h);
    }
    if let Some((animation, offset)) = flipbook {
        e.insert(UvFlipbook {
            animation: animation.clone(),
            offset: *offset,
            material: part.material.clone(),
        });
        stats.flipbooks += 1;
    }
}

/// Plays node animation (SOD and M3D): a step flipbook, evenly spaced keys.
pub fn animate(clock: Res<crate::clock::AnimClock>, mut q: Query<(&NodeAnim, &mut Transform)>) {
    let t = clock.wall() as f32;
    for (anim, mut tf) in &mut q {
        let n = match &anim.keys {
            AnimKeys::Matrix(k) => k.len(),
            AnimKeys::Scale(k) => k.len(),
        };
        let phase = (t % anim.period) / anim.period;
        let k = ((phase * n as f32) as usize).min(n - 1);
        *tf = match &anim.keys {
            AnimKeys::Matrix(keys) => keys[k],
            AnimKeys::Scale(keys) => anim.base.with_scale(Vec3::splat(keys[k])),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bevy's shading of a normal-map texel on a quad, done on the CPU exactly as
    /// `apply_normal_mapping` does: N' = x*T + y*B + z*N with B = w * cross(N, T).
    /// The quad lies in the XY plane facing +Z; +u runs along +X, +v (down the
    /// image) along -Y.
    fn shade(texel: [u8; 3]) -> Vec3 {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default(),
        )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0],
            ],
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        )
            .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
        m.generate_tangents().unwrap();
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(t)) = m.attribute(Mesh::ATTRIBUTE_TANGENT) else {
            panic!("no tangents")
        };
        let tan = Vec3::new(t[0][0], t[0][1], t[0][2]);
        let n = Vec3::Z;
        let b = t[0][3] * n.cross(tan);
        let nt = Vec3::new(texel[0] as f32, texel[1] as f32, texel[2] as f32) / 255.0 * 2.0 - 1.0;
        (nt.x * tan + nt.y * b + nt.z * n).normalize()
    }

    fn texel_from_height(f: impl Fn(u32, u32) -> u8) -> [u8; 3] {
        let mut img = sta_assets::Rgba {
            width: 8,
            height: 8,
            has_alpha: false,
            pixels: vec![0; 8 * 8 * 4],
        };
        for y in 0..8 {
            for x in 0..8 {
                let h = f(x, y);
                img.pixels[((y * 8 + x) * 4) as usize..][..4].copy_from_slice(&[h, h, h, 255]);
            }
        }
        let n = sta_assets::pixels::height_to_normal(&img);
        let i = ((3 * 8 + 3) * 4) as usize;
        [n.pixels[i], n.pixels[i + 1], n.pixels[i + 2]]
    }

    #[test]
    fn planet_atmosphere_shells_are_see_through() {
        let g = |b: &str| Group { material: String::new(), blend: b.into(), maps: Default::default(), faces: vec![] };
        // FO's planet SODs: the cloud shell over the ground must not hide it.
        assert!(matches!(blend(&g("atmosphere")), (AlphaMode::Blend, false)));
        assert!(matches!(blend(&g("default")), (AlphaMode::Opaque, false)));
        assert!(matches!(blend(&g("Additive")), (AlphaMode::Add, true)));
    }

    #[test]
    fn height_maps_tilt_away_from_the_slope() {
        // Height rising toward +u (world +X): the lit normal must lean toward -X.
        let n = shade(texel_from_height(|x, _| (x * 30) as u8));
        assert!(n.x < -0.1 && n.y.abs() < 0.05, "u slope gave {n}");
        // Height rising down the image (+v, world -Y): the normal must lean toward +Y.
        let n = shade(texel_from_height(|_, y| (y * 30) as u8));
        assert!(n.y > 0.1 && n.x.abs() < 0.05, "v slope gave {n}");
    }

    #[test]
    fn direct3d_normal_maps_keep_their_meaning() {
        // A Direct3D (green-down) normal map texel leaning toward image-down (+v,
        // world -Y): after our conversion the lit normal must lean toward -Y.
        let mut img = sta_assets::Rgba {
            width: 1,
            height: 1,
            has_alpha: false,
            pixels: vec![128, 200, 220, 255],
        };
        sta_assets::pixels::convert_direct3d_normal_map(&mut img);
        let n = shade([img.pixels[0], img.pixels[1], img.pixels[2]]);
        assert!(n.y < -0.1, "D3D +v texel gave {n}");
    }

    #[test]
    fn matrix_conversion_mirrors_z() {
        // Translation (1, 2, 3) with identity axes.
        let m = Matrix34 {
            m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0],
        };
        let t = to_bevy(&m);
        assert_eq!(t.translation, Vec3::new(1.0, 2.0, -3.0));
        // A point transformed in SOD space then mirrored equals mirror then Bevy transform.
        let r = Matrix34 {
            m: [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 5.0, 0.0, 7.0],
        };
        let p = [1.0, 2.0, 3.0];
        let expect = Vec3::from(mirror(r.transform_point(p)));
        let got = to_bevy(&r).transform_point(Vec3::from(mirror(p)));
        assert!((expect - got).length() < 1e-5, "{expect} vs {got}");
    }
}
