//! Turn a parsed SOD into a Bevy entity hierarchy.
//!
//! Coordinate conversion: SOD is Direct3D left-handed with row-vector matrices;
//! Bevy is right-handed. Positions and transforms are mirrored on Z
//! (`M' = S M S`, `S = diag(1, 1, -1)`). Triangle winding is kept as stored:
//! with the mirror applied, the file's order is front-facing in Bevy.

use crate::sprites::{self, Billboard, SpriteFx, UvFlipbook};
use crate::textures::Loader;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_assets::spr_anim;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::{Face, PrimitiveTopology};
use sta_assets::a2fmt::Matrix34;
use sta_assets::a2fmt::sod::{ChannelKeys, Mesh as SodMesh, NodeKind, Role, Sod};
use std::collections::{HashMap, HashSet};

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

/// Face groups that take the owner's team colour.
#[derive(Component)]
pub struct TeamPart;

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

/// Lower-case blend mode from the mesh's texture material.
fn blend(mesh: &SodMesh) -> (AlphaMode, bool) {
    match mesh.texture_material.to_ascii_lowercase().as_str() {
        "additive" | "wormhole" => (AlphaMode::Add, true),
        "alphathreshold" => (AlphaMode::Mask(0.5), false),
        "translucent" | "alpha" | "transparent" => (AlphaMode::Blend, false),
        _ => (AlphaMode::Opaque, false),
    }
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
}

/// Spawn `sod` under `root`. Only the finest LOD level of each LOD switch is shown.
pub fn spawn(ctx: &mut SpawnCtx, sod: &Sod, root: Entity) -> SpawnStats {
    let mut stats = SpawnStats::default();

    // Hide every LOD level except the most detailed.
    let mut hidden: HashSet<usize> = HashSet::new();
    for (i, n) in sod.nodes.iter().enumerate() {
        if n.role() == Role::Geometry {
            let levels = sod.lod_levels(i);
            if let Some((_, coarser)) = levels.split_last() {
                hidden.extend(coarser.iter().copied());
            }
        }
    }

    let anims: HashMap<String, &sta_assets::a2fmt::sod::AnimChannel> =
        sod.channels.iter().map(|c| (c.node.to_ascii_lowercase(), c)).collect();
    // Type 4 references: a texture flipbook on a mesh's UV offset.
    let flipbooks: HashMap<String, (String, f32)> = sod
        .anim_refs
        .iter()
        .filter(|r| r.ref_type == 4)
        .map(|r| (r.node.to_ascii_lowercase(), (r.animation.clone(), r.offset)))
        .collect();
    let quad = ctx.meshes.add(sprites::unit_quad());

    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);

    let mut entities: Vec<Option<Entity>> = vec![None; sod.nodes.len()];
    // Parents precede children only by chance in files; walk the tree from roots.
    let mut stack: Vec<usize> = sod.roots().collect::<Vec<_>>();
    stack.reverse();
    while let Some(i) = stack.pop() {
        let node = &sod.nodes[i];
        if hidden.contains(&i) {
            continue;
        }
        let parent = node.parent.and_then(|p| entities[p]).unwrap_or(root);
        let base = to_bevy(&node.transform);
        let mut e = ctx.commands.spawn((Name::new(node.name.clone()), base, Visibility::default(), ChildOf(parent)));

        if let Some(ch) = anims.get(&node.name.to_ascii_lowercase())
            && ch.key_count() > 1
            && ch.period > 0.0
        {
            let keys = match &ch.keys {
                ChannelKeys::Matrix(k) => AnimKeys::Matrix(k.iter().map(to_bevy).collect()),
                ChannelKeys::Scale(k) => AnimKeys::Scale(k.clone()),
            };
            e.insert(NodeAnim {
                period: ch.period,
                base,
                keys,
            });
            stats.animated_nodes += 1;
        }
        let id = e.id();
        entities[i] = Some(id);

        if let NodeKind::Mesh(mesh) = &node.kind {
            let world = sod.model_transform(i);
            for p in &mesh.positions {
                let w = Vec3::from(mirror(world.transform_point(*p)));
                min = min.min(w);
                max = max.max(w);
            }
            let flip = flipbooks.get(&node.name.to_ascii_lowercase());
            spawn_mesh(ctx, sod, mesh, id, flip, &mut stats);
        }
        match &node.kind {
            NodeKind::Sprite => spawn_sprite_node(ctx, node.sprite_node_id(), id, &quad, &mut stats),
            NodeKind::Emitter { emitter } => match crate::particles::Template::new(ctx.sprites, emitter) {
                Some(tpl) => {
                    ctx.commands.entity(id).insert(crate::particles::Emitter {
                        tpl: std::sync::Arc::new(tpl),
                        owner: root,
                        timer: 0.0,
                    });
                    stats.emitters += 1;
                }
                None => stats.unknown_sprite_nodes.push(format!("emitter {emitter}")),
            },
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

fn spawn_sprite_node(ctx: &mut SpawnCtx, id: &str, parent: Entity, quad: &Handle<Mesh>, stats: &mut SpawnStats) {
    let table = ctx.sprites;
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
    stats.sprite_nodes += 1;
}

fn spawn_mesh(
    ctx: &mut SpawnCtx,
    sod: &Sod,
    mesh: &SodMesh,
    parent: Entity,
    flipbook: Option<&(String, f32)>,
    stats: &mut SpawnStats,
) {
    // Before 1.7 the texture comes from the file's texture-material table.
    let tex_name = mesh.diffuse_texture().map(str::to_owned).or_else(|| {
        sod.texture_materials
            .iter()
            .find(|t| t.ident.eq_ignore_ascii_case(&mesh.texture_material))
            .map(|t| t.texture.clone())
    });
    let texture = tex_name.as_deref().and_then(|n| {
        let h = ctx.textures.colour(n, ctx.borg);
        if h.is_none() {
            stats.missing_textures.push(n.to_owned());
        }
        h
    });
    // SOD 1.93 bump path: a height map turned into a normal map.
    let normal_map = mesh.height_map().and_then(|n| {
        let h = ctx.textures.normal_from_height(n);
        if h.is_none() {
            stats.missing_textures.push(n.to_owned());
        }
        h
    });
    if normal_map.is_some() {
        stats.bump_meshes += 1;
    }
    let (alpha_mode, additive) = blend(mesh);
    let two_sided = mesh.cull == 0;

    for group in &mesh.groups {
        if group.faces.is_empty() {
            continue;
        }
        // De-index this group: one vertex per distinct (position, uv) corner.
        let mut remap: HashMap<(u16, u16), u32> = HashMap::new();
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut indices: Vec<u32> = Vec::with_capacity(group.faces.len() * 3);
        for face in &group.faces {
            // File winding is kept: after the Z mirror it is already
            // counter-clockwise (front-facing) for Bevy. Reversing it turned
            // FO4 hulls inside out in the viewer.
            for c in face {
                let idx = *remap.entry((c.position, c.uv)).or_insert_with(|| {
                    positions.push(mirror(mesh.positions[c.position as usize]));
                    uvs.push(mesh.uvs[c.uv as usize]);
                    (positions.len() - 1) as u32
                });
                indices.push(idx);
            }
        }
        // Area-weighted normals by shared position (the file stores none).
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
            .map(|p| by_pos[&key(*p)].normalize_or(Vec3::Y).to_array())
            .collect();

        stats.triangles += indices.len() / 3;
        stats.meshes += 1;

        let mut bevy_mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(indices));
        let mut group_normal_map = normal_map.clone();
        if group_normal_map.is_some() && bevy_mesh.generate_tangents().is_err() {
            group_normal_map = None;
        }

        let lm = sod.lighting_material(&group.material);
        let team = group.material.to_ascii_lowercase().starts_with("team");
        let self_illum = lm.is_some_and(|m| m.self_illuminating);
        let base_color = if team { ctx.team } else { Color::WHITE };
        let material = StandardMaterial {
            base_color,
            base_color_texture: texture.clone(),
            normal_map_texture: group_normal_map,
            alpha_mode,
            unlit: additive || self_illum,
            double_sided: two_sided,
            cull_mode: if two_sided { None } else { Some(Face::Back) },
            perceptual_roughness: 0.6,
            reflectance: 0.3,
            ..default()
        };
        let material = ctx.materials.add(material);
        let mut e = ctx.commands.spawn((
            Mesh3d(ctx.meshes.add(bevy_mesh)),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            ChildOf(parent),
        ));
        if team {
            e.insert(TeamPart);
        }
        if let Some((animation, offset)) = flipbook {
            e.insert(UvFlipbook {
                animation: animation.clone(),
                offset: *offset,
                material,
            });
            stats.flipbooks += 1;
        }
    }
}

/// Plays SOD node animation: a step flipbook, evenly spaced keys.
pub fn animate(time: Res<Time>, mut q: Query<(&NodeAnim, &mut Transform)>) {
    let t = time.elapsed_secs();
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
