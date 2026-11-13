//! An engine-independent model that every model format loads into.
//!
//! SOD and M3D are read straight into this type; neither is converted to the
//! other. Transforms and positions stay in the files' Direct3D left-handed
//! convention (row vectors, `Matrix34`); the renderer converts once. Triangle
//! corners are stored in SOD winding order, so one builder handles every format.

use a2fmt::Matrix34;
use a2fmt::sod::{ChannelKeys, NodeKind as SodKind, Sod};

pub mod pbr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Sod,
    M3d,
    Gltf,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub format: SourceFormat,
    pub version: f32,
    pub materials: Vec<LightingMaterial>,
    pub nodes: Vec<Node>,
    pub channels: Vec<Channel>,
    /// Texture flipbooks: (mesh node name, `@animation` id, time offset).
    pub flipbooks: Vec<(String, String, f32)>,
    /// Notes from loading (things kept as plain transforms, skipped channels...).
    pub warnings: Vec<String>,
    /// Native PBR materials/images/attributes. Absent for unchanged legacy data.
    pub pbr: Option<pbr::PbrModel>,
}

#[derive(Debug, Clone)]
pub struct LightingMaterial {
    pub name: String,
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub power: f32,
    pub self_illuminating: bool,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Local transform in the parent's space.
    pub transform: Matrix34,
    pub kind: NodeKind,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Null,
    /// An authored Legacy LOD control. M1 displays its finest geometry branch;
    /// camera-distance switching is a separate renderer policy.
    Lod,
    /// Parsed legacy light data. The subtype remains source-specific; keeping
    /// these values does not imply the renderer reproduces its light shader.
    Light(a2fmt::sod::Light),
    Mesh(Mesh),
    /// The node name up to its first underscore is a `@sprite_node` id.
    Sprite,
    /// An `@emitter` id.
    Emitter(String),
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub groups: Vec<Group>,
    pub two_sided: bool,
}

/// Texture maps a face group uses, by texture name (no extension).
#[derive(Debug, Clone, Default)]
pub struct Maps {
    pub diffuse: Option<String>,
    /// Height map (SOD bump path): converted to a normal map.
    pub height: Option<String>,
    /// M3D bump map: a normal map or a height map, decided from its pixels.
    pub bump: Option<String>,
    pub specular: Option<String>,
    pub glow: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Group {
    /// Lighting material name (`team*` takes the team colour).
    pub material: String,
    /// Blend mode: `default`, `opaque`, `additive`, `alpha`, `alphathreshold`, ...
    pub blend: String,
    pub maps: Maps,
    /// Corners as (position index, uv index), SOD winding.
    pub faces: Vec<[(u32, u32); 3]>,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub node: String,
    pub period: f32,
    pub keys: Keys,
}

#[derive(Debug, Clone)]
pub enum Keys {
    Matrix(Vec<Matrix34>),
    Scale(Vec<f32>),
}

impl Node {
    pub fn is_geometry(&self) -> bool {
        matches!(self.kind, NodeKind::Lod) || self.name.to_ascii_lowercase().starts_with("geometry")
    }
    pub fn is_lod(&self) -> bool {
        self.name.to_ascii_lowercase().starts_with("lod")
    }
    /// The `@sprite_node` id a sprite node draws: its name up to the first
    /// underscore, leading underscores skipped (the engine's `strtok`).
    pub fn sprite_node_id(&self) -> &str {
        let n = self.name.trim_start_matches('_');
        n.split('_').next().unwrap_or(n)
    }
    pub fn mesh(&self) -> Option<&Mesh> {
        match &self.kind {
            NodeKind::Mesh(m) => Some(m),
            _ => None,
        }
    }
}

impl Model {
    pub fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(|&i| self.nodes[i].parent.is_none())
    }

    pub fn lighting_material(&self, name: &str) -> Option<&LightingMaterial> {
        self.materials.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// The node's transform in model space.
    pub fn model_transform(&self, mut node: usize) -> Matrix34 {
        let mut m = self.nodes[node].transform;
        let mut guard = 0;
        while let Some(p) = self.nodes[node].parent {
            m = m.then(&self.nodes[p].transform);
            node = p;
            guard += 1;
            if guard > self.nodes.len() {
                break;
            }
        }
        m
    }

    fn face_count(&self, node: usize) -> usize {
        let own = self.nodes[node].mesh().map_or(0, |m| m.groups.iter().map(|g| g.faces.len()).sum());
        own + self.nodes[node].children.iter().map(|&c| self.face_count(c)).sum::<usize>()
    }

    /// The damage-state nodes the engine hides on a healthy craft: the first node
    /// named after each system (`shields`, `engines`, `target`, `sensors`, `life`,
    /// matched as `CraftClass::InitializeDamageNodes` finds them), plus the
    /// `damage` and `crew` groups (hull breaches, fires, venting).
    pub fn damage_nodes(&self) -> Vec<usize> {
        if self.format == SourceFormat::Gltf { return Vec::new(); }
        ["shields", "engines", "target", "sensors", "life", "damage", "crew"]
            .iter()
            .filter_map(|want| self.nodes.iter().position(|n| n.name.eq_ignore_ascii_case(want)))
            .collect()
    }

    /// The `borg` node: shown only while the craft is assimilated.
    pub fn borg_node(&self) -> Option<usize> {
        if self.format == SourceFormat::Gltf { return None; }
        self.nodes.iter().position(|n| n.name.eq_ignore_ascii_case("borg"))
    }

    /// Nodes to hide so only the most detailed level of each LOD switch shows:
    /// every `lod*` child of a `geometry*` node except the one with most faces.
    pub fn coarse_lods(&self) -> Vec<usize> {
        if self.format == SourceFormat::Gltf { return Vec::new(); }
        let mut out = Vec::new();
        for n in self.nodes.iter().filter(|n| n.is_geometry()) {
            let mut levels: Vec<usize> = n.children.iter().copied().filter(|&c| self.nodes[c].is_lod()).collect();
            levels.sort_by_key(|&c| self.face_count(c));
            levels.pop();
            out.extend(levels);
        }
        out
    }

    pub fn from_sod(sod: &Sod) -> Model {
        let nodes = sod
            .nodes
            .iter()
            .map(|n| Node {
                name: n.name.clone(),
                parent: n.parent,
                children: n.children.clone(),
                transform: n.transform,
                kind: match &n.kind {
                    SodKind::Mesh(m) => {
                        // Before 1.7 the texture comes from the file's material table.
                        let diffuse = m.diffuse_texture().map(str::to_owned).or_else(|| {
                            sod.texture_materials
                                .iter()
                                .find(|t| t.ident.eq_ignore_ascii_case(&m.texture_material))
                                .map(|t| t.texture.clone())
                        });
                        let maps = Maps {
                            diffuse,
                            height: m.height_map().map(str::to_owned),
                            ..Default::default()
                        };
                        // Before 1.7 the name is an ident of the file's texture-material
                        // table, not a blend mode: such a mesh draws as `default` (the
                        // alpha-tested reading of its mode bytes is a hull matter, RND-24).
                        let blend = if sod.texture_materials.iter().any(|t| t.ident.eq_ignore_ascii_case(&m.texture_material)) {
                            "default".to_owned()
                        } else {
                            m.texture_material.to_ascii_lowercase()
                        };
                        NodeKind::Mesh(Mesh {
                            positions: m.positions.clone(),
                            uvs: m.uvs.clone(),
                            groups: m
                                .groups
                                .iter()
                                .map(|g| Group {
                                    material: g.material.clone(),
                                    blend: blend.clone(),
                                    maps: maps.clone(),
                                    faces: g
                                        .faces
                                        .iter()
                                        .map(|f| f.map(|c| (c.position as u32, c.uv as u32)))
                                        .collect(),
                                })
                                .collect(),
                            two_sided: m.cull == 0,
                        })
                    }
                    SodKind::Sprite => NodeKind::Sprite,
                    SodKind::Emitter { emitter } => NodeKind::Emitter(emitter.clone()),
                    SodKind::Light(light) => NodeKind::Light(light.clone()),
                    _ => NodeKind::Null,
                },
            })
            .collect();
        Model {
            format: SourceFormat::Sod,
            version: sod.version,
            materials: sod
                .lighting_materials
                .iter()
                .map(|m| LightingMaterial {
                    name: m.name.clone(),
                    diffuse: m.diffuse,
                    specular: m.specular,
                    power: m.power,
                    self_illuminating: m.self_illuminating,
                })
                .collect(),
            nodes,
            channels: sod
                .channels
                .iter()
                .map(|c| Channel {
                    node: c.node.clone(),
                    period: c.period,
                    keys: match &c.keys {
                        ChannelKeys::Matrix(k) => Keys::Matrix(k.clone()),
                        ChannelKeys::Scale(k) => Keys::Scale(k.clone()),
                    },
                })
                .collect(),
            flipbooks: sod
                .anim_refs
                .iter()
                .filter(|r| r.ref_type == 4)
                .map(|r| (r.node.clone(), r.animation.clone(), r.offset))
                .collect(),
            warnings: Vec::new(),
            pbr: None,
        }
    }

    /// Model-space bounds of all meshes, or `None` if there are none.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let mut any = false;
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(m) = n.mesh() {
                let w = self.model_transform(i);
                for p in &m.positions {
                    let q = w.transform_point(*p);
                    for a in 0..3 {
                        min[a] = min[a].min(q[a]);
                        max[a] = max[a].max(q[a]);
                    }
                    any = true;
                }
            }
        }
        any.then_some((min, max))
    }
}
