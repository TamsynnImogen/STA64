//! Star Trek: Legacy `.m3d` models, loaded directly into the shared model type.
//!
//! Parser and hierarchy clean-up ported from A2FOHookExtensions
//! `rust/a2fo_core/src/m3d.rs` (commit 7eaa2c7),
//! the same author's code. The SOD writer and C FFI layer are not ported: STA64
//! reads M3D directly through [`load`].
// Keep the legacy field order, bounded indices and ABI-shaped arguments auditable.
#![allow(
    clippy::needless_range_loop,
    clippy::field_reassign_with_default,
    clippy::unnecessary_filter_map
)]
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::ToString,
    vec,
    vec::Vec,
};

pub mod chunks;

type Bytes = Vec<u8>;
fn b(s: &str) -> Bytes {
    s.as_bytes().to_vec()
}
fn join(parts: &[&[u8]]) -> Bytes {
    let mut out = Vec::new();
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}
fn num(n: usize) -> Bytes {
    n.to_string().into_bytes()
}
fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}
fn at_least(v: f32, t: f32) -> bool {
    v > t - 0.005
}
fn above(v: f32, t: f32) -> bool {
    v > t + 0.005
}
fn lower_ascii(s: &[u8]) -> Bytes {
    s.iter()
        .map(|c: &u8| if c.is_ascii_uppercase() { c + 32 } else { *c })
        .collect()
}
fn authored(n: &Node) -> Bytes {
    lower_ascii(if n.source_name.is_empty() {
        &n.name
    } else {
        &n.source_name
    })
}

#[derive(Default, Clone)]
struct Material {
    name: Bytes,
    ambient: [f32; 3],
    diffuse: [f32; 3],
    specular: [f32; 3],
    shininess: f32,
    flags: u32,
    flags2: u32,
}
#[derive(Default, Clone)]
struct Face {
    positions: [u32; 3],
    normals: [u32; 3],
    uvs: [u32; 3],
}
#[derive(Default, Clone)]
struct Group {
    material: Bytes,
    diffuse: Bytes,
    bump: Bytes,
    specular: Bytes,
    glow: Bytes,
    detail: Bytes,
    shader: Bytes,
    extra_shader: Bytes,
    texture_flags: [u32; 5],
    faces: Vec<Face>,
}
#[derive(Clone)]
struct Mesh {
    flags: u32,
    trailer: u16,
    vertices: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    groups: Vec<Group>,
    cull: i8,
}
impl Default for Mesh {
    fn default() -> Self {
        Self {
            flags: 0,
            trailer: 0,
            vertices: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            groups: Vec::new(),
            cull: 1,
        }
    }
}
#[derive(Clone)]
struct Node {
    kind: u16,
    source_id: u32,
    source_parent_id: u32,
    numeric: bool,
    source_name: Bytes,
    name: Bytes,
    parent: Bytes,
    matrix: [f32; 12],
    mesh: Mesh,
    payload: Bytes,
    light: Option<a2fmt::sod::Light>,
}
impl Default for Node {
    fn default() -> Self {
        Self {
            kind: 0,
            source_id: 0,
            source_parent_id: 0,
            numeric: false,
            source_name: Vec::new(),
            name: Vec::new(),
            parent: Vec::new(),
            matrix: [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.],
            mesh: Mesh::default(),
            payload: Vec::new(),
            light: None,
        }
    }
}
#[derive(Default, Clone)]
struct Animation {
    node: Bytes,
    kind: u8,
    period: f32,
    matrices: Vec<[f32; 12]>,
    scalars: Vec<f32>,
    supported: bool,
}
#[derive(Default, Clone)]
struct Reference {
    kind: i8,
    node: Bytes,
    animation: Bytes,
    extra: u32,
}
#[derive(Default, Clone)]
struct File {
    version: f32,
    materials: Vec<Material>,
    nodes: Vec<Node>,
    animations: Vec<Animation>,
    references: Vec<Reference>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    error: Bytes,
    strict_numbers: bool,
    preserve_numbers: bool,
}
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            error: Vec::new(),
            strict_numbers: false,
            preserve_numbers: false,
        }
    }
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }
    fn raw(&mut self, n: usize, what: &str) -> Option<&'a [u8]> {
        if n > self.remaining() {
            self.error = b(&format!(
                "Unexpected end of M3D at byte {} while reading {}.",
                self.at, what
            ));
            return None;
        }
        let p = self.at;
        self.at += n;
        Some(&self.data[p..self.at])
    }
    fn skip(&mut self, n: usize, what: &str) -> Option<()> {
        self.raw(n, what).map(|_| ())
    }
    fn u8(&mut self, what: &str) -> Option<u8> {
        Some(self.raw(1, what)?[0])
    }
    fn i8(&mut self, what: &str) -> Option<i8> {
        Some(self.u8(what)? as i8)
    }
    fn u16(&mut self, what: &str) -> Option<u16> {
        Some(u16::from_le_bytes(self.raw(2, what)?.try_into().ok()?))
    }
    fn u32(&mut self, what: &str) -> Option<u32> {
        Some(u32::from_le_bytes(self.raw(4, what)?.try_into().ok()?))
    }
    fn f32(&mut self, what: &str) -> Option<f32> {
        let value = f32::from_bits(self.u32(what)?);
        if self.strict_numbers && !value.is_finite() {
            return self.fail(&format!("Non-finite M3D {what} at byte {}.", self.at - 4));
        }
        Some(value)
    }
    fn id(&mut self, what: &str) -> Option<Bytes> {
        let n = self.u16(what)? as usize;
        if n > self.remaining() {
            self.error = b(&format!(
                "Invalid M3D identifier length while reading {}.",
                what
            ));
            return None;
        }
        let mut s = self.raw(n, what)?.to_vec();
        while s.last() == Some(&0) {
            s.pop();
        }
        Some(s)
    }
    fn fail<T>(&mut self, s: &str) -> Option<T> {
        self.error = b(s);
        None
    }
    fn vec3(&mut self, what: &str) -> Option<[f32; 3]> {
        let mut a = [0.; 3];
        for v in &mut a {
            let value = self.f32(what)?;
            *v = if self.preserve_numbers { value } else { finite(value) };
        }
        Some(a)
    }
    fn texture(&mut self, what: &str) -> Option<(Bytes, u32)> {
        let name = self.id(what)?;
        let flags = self.u32("texture flags")?;
        Some((name, flags))
    }
    fn index(&mut self, wide: bool, what: &str) -> Option<u32> {
        if wide { self.u32(what) } else { self.u16(what).map(u32::from) }
    }
    fn material_record(&mut self, version: f32) -> Option<Group> {
        let mut g = Group::default();
        g.material = self.id("group material")?;
        if at_least(version, 1.99) {
            (g.diffuse, g.texture_flags[0]) = self.texture("diffuse texture")?;
            (g.bump, g.texture_flags[1]) = self.texture("bump texture")?;
            (g.specular, g.texture_flags[2]) = self.texture("specular texture")?;
            (g.glow, g.texture_flags[3]) = self.texture("glow texture")?;
            if at_least(version, 2.0) {
                (g.detail, g.texture_flags[4]) = self.texture("detail texture")?;
            }
        } else {
            let Some(n) = self.u32("group texture count") else {
                return self.fail("Invalid M3D group texture count.");
            };
            if n > 1024 {
                return self.fail("Invalid M3D group texture count.");
            }
            for i in 0..n {
                let (name, kind) = self.texture("group texture")?;
                if let Some(flags) = g.texture_flags.get_mut(i as usize) { *flags = kind; }
                match i {
                    0 => g.diffuse = name,
                    1 => g.bump = name,
                    2 => g.specular = name,
                    3 => g.glow = name,
                    4 => g.detail = name,
                    _ => (),
                }
                if kind & 0x200 != 0 {
                    self.f32("legacy bump strength")?;
                }
            }
        }
        g.shader = self.id("group shader")?;
        if at_least(version, 2.05) {
            g.extra_shader = self.id("group extra shader data")?;
        }
        Some(g)
    }
    fn group(&mut self, version: f32, mesh_flags: u32, inline_material: bool) -> Option<Group> {
        let faces = self.u16("group face count")? as usize;
        let mut g = if inline_material { self.material_record(version)? } else { Group::default() };
        g.faces = vec![Face::default(); faces];
        if !at_least(version, 1.97) {
            for f in &mut g.faces {
                for c in 0..3 {
                    f.positions[c] = u32::from(self.u16("position index")?);
                    f.normals[c] = f.positions[c];
                    f.uvs[c] = u32::from(self.u16("UV index")?);
                }
            }
            return Some(g);
        }
        let wide = at_least(version, 2.06);
        let mut corners = vec![[0u32; 3]; faces];
        for f in &mut corners {
            for v in f {
                *v = self.index(wide, "corner record index")?;
            }
        }
        let Some(n) = self.u32("corner record count") else {
            return self.fail("Invalid M3D corner record count.");
        };
        if n > 10_000_000 {
            return self.fail("Invalid M3D corner record count.");
        }
        let stride = (if at_least(version, 2.02) { 3 } else { 2 }) * (if wide { 4 } else { 2 });
        if n as usize > self.remaining() / stride {
            return self.fail("Truncated M3D corner records.");
        }
        let mut records = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let pos;
            let normal;
            let uv;
            if at_least(version, 2.02) {
                pos = self.index(wide, "corner attribute index")?;
                normal = self.index(wide, "corner attribute index")?;
                uv = self.index(wide, "corner attribute index")?;
            } else {
                pos = u32::from(self.u16("position index")?);
                normal = pos;
                uv = u32::from(self.u16("UV index")?);
            }
            records.push((pos, normal, uv));
        }
        for (f, indices) in g.faces.iter_mut().zip(&corners) {
            for c in 0..3 {
                let i = indices[c] as usize;
                if i >= records.len() {
                    return self.fail("M3D face refers to an out-of-range corner record.");
                }
                f.positions[c] = records[i].0;
                f.normals[c] = records[i].1;
                f.uvs[c] = records[i].2;
            }
        }
        if mesh_flags & 2 != 0 {
            self.skip(n as usize * 24, "tangent data")?;
        }
        if version >= 1.9925 {
            self.skip(24, "group bounds")?;
            let nr = self.u32("bounds record count")?;
            if nr > 10_000_000 {
                return self.fail("Invalid M3D bounds record count.");
            }
            self.skip(nr as usize * 8, "bounds records")?;
            let ni = self.u32("bounds index count")?;
            if ni > 10_000_000 {
                return self.fail("Invalid M3D bounds index count.");
            }
            self.skip(ni as usize * 2, "bounds indices")?;
            self.u16("bounds depth")?;
        }
        Some(g)
    }
    fn mesh(&mut self, version: f32) -> Option<Mesh> {
        self.mesh_with_materials(version, true)
    }
    fn mesh_with_materials(&mut self, version: f32, inline_material: bool) -> Option<Mesh> {
        let mut m = Mesh::default();
        m.flags = self.u32("mesh flags")?;
        m.trailer = self.u16("mesh trailer")?;
        let nv = self.u16("vertex count")?;
        let mut nn = nv;
        if at_least(version, 2.02) {
            nn = self.u16("normal count")?;
        }
        let nu = self.u16("UV count")?;
        let ng = self.u16("group count")?;
        for _ in 0..nv {
            m.vertices.push(self.vec3("vertex")?);
        }
        if m.flags & 0x10 != 0 {
            for _ in 0..nn {
                m.normals.push(self.vec3("normal")?);
            }
        }
        for _ in 0..nu {
            let mut uv = [self.f32("UV")?, self.f32("UV")?];
            if !self.preserve_numbers {
                uv[0] = finite(uv[0]);
                uv[1] = finite(uv[1]);
            }
            m.uvs.push(uv);
        }
        for _ in 0..ng {
            m.groups.push(self.group(version, m.flags, inline_material)?);
        }
        m.cull = self.i8("cull flag")?;
        if !at_least(version, 2.01) {
            let n = self.u16("mesh extra count")?;
            self.skip(n as usize * 2, "mesh extra data")?;
        }
        Some(m)
    }
}

fn parse(data: &[u8], fmt: &impl Fn(u32) -> Bytes) -> Result<File, Bytes> {
    let mut r = Reader::new(data);
    let mut f = File::default();
    let h = r.raw(10, "header");
    if h.is_none() {
        return Err(r.error);
    }
    if &h.unwrap()[..8] != b"Mad3D_SW" {
        return Err(b("Not a Star Trek Legacy M3D stream."));
    }
    f.version = r.f32("version").ok_or_else(|| r.error.clone())?;
    if !at_least(f.version, 1.94) || above(f.version, 2.06) {
        return Err(join(&[
            b"Unsupported Legacy M3D version ",
            &fmt(f.version.to_bits()),
            b"; supported native layouts are 1.94 through 2.06.",
        ]));
    }
    let n = r.u16("material count").ok_or_else(|| r.error.clone())?;
    for _ in 0..n {
        let mut m = Material::default();
        m.name = r.id("material name").ok_or_else(|| r.error.clone())?;
        m.ambient = r.vec3("ambient colour").ok_or_else(|| r.error.clone())?;
        m.diffuse = r.vec3("diffuse colour").ok_or_else(|| r.error.clone())?;
        m.specular = r.vec3("specular colour").ok_or_else(|| r.error.clone())?;
        m.shininess = r.f32("shininess").ok_or_else(|| r.error.clone())?;
        if at_least(f.version, 1.99) {
            m.flags = r.u32("material flags").ok_or_else(|| r.error.clone())?;
            m.flags2 = r.u32("material flags2").ok_or_else(|| r.error.clone())?;
        } else {
            m.flags = r.u8("material flags").ok_or_else(|| r.error.clone())? as u32;
            m.flags2 = r.u8("material flags2").ok_or_else(|| r.error.clone())? as u32;
        }
        f.materials.push(m);
    }
    let n = r.u16("node count").ok_or_else(|| r.error.clone())?;
    for _ in 0..n {
        let mut node = Node::default();
        if at_least(f.version, 1.96) {
            node.numeric = true;
            node.source_name = r.id("node name").ok_or_else(|| r.error.clone())?;
            node.source_id = r.u32("node ID").ok_or_else(|| r.error.clone())?;
            node.kind = r.u16("node type").ok_or_else(|| r.error.clone())?;
            node.source_parent_id = r.u32("parent ID").ok_or_else(|| r.error.clone())?;
            node.name = node.source_name.clone();
        } else {
            node.kind = r.u16("node type").ok_or_else(|| r.error.clone())?;
            node.name = r.id("node name").ok_or_else(|| r.error.clone())?;
            node.parent = r.id("parent name").ok_or_else(|| r.error.clone())?;
            node.source_name = node.name.clone();
        }
        for v in &mut node.matrix {
            *v = r.f32("node matrix").ok_or_else(|| r.error.clone())?;
        }
        if node.matrix.iter().any(|v| !v.is_finite()) {
            node.matrix = Node::default().matrix;
        }
        match node.kind {
            1 => node.mesh = r.mesh(f.version).ok_or_else(|| r.error.clone())?,
            2 => {
                let kind = r.u8("light subtype").ok_or_else(|| r.error.clone())?;
                let mut values = [0.0; 5];
                for value in &mut values {
                    *value = r.f32("light data").ok_or_else(|| r.error.clone())?;
                }
                if values.iter().any(|value| !value.is_finite()) {
                    return Err(b("Non-finite M3D light data."));
                }
                node.light = Some(a2fmt::sod::Light {
                    kind,
                    colour: [values[0], values[1], values[2]],
                    falloff_start: values[3],
                    falloff_range: values[4],
                });
            }
            3 | 12 => node.payload = r.id("node command").ok_or_else(|| r.error.clone())?,
            _ => (),
        }
        f.nodes.push(node);
    }
    if at_least(f.version, 1.96) {
        let mut ids = BTreeMap::new();
        let mut names = BTreeMap::new();
        for (i, node) in f.nodes.iter().enumerate() {
            ids.insert(node.source_id, i);
            *names.entry(node.source_name.clone()).or_insert(0usize) += 1;
        }
        for node in &mut f.nodes {
            if names[&node.source_name] > 1 || node.source_name.is_empty() {
                node.name = join(&[
                    if node.source_name.is_empty() {
                        b"m3d_node"
                    } else {
                        &node.source_name
                    },
                    b"_m3d_",
                    &num(node.source_id as usize),
                ]);
            } else {
                node.name = node.source_name.clone();
            }
        }
        let has_zero = ids.contains_key(&0);
        let links: Vec<Bytes> = f
            .nodes
            .iter()
            .map(|n| {
                if n.source_parent_id == 0 && !has_zero {
                    return Vec::new();
                }
                match ids.get(&n.source_parent_id) {
                    Some(i) if f.nodes[*i].source_id != n.source_id => f.nodes[*i].name.clone(),
                    _ => Vec::new(),
                }
            })
            .collect();
        for (n, p) in f.nodes.iter_mut().zip(links) {
            n.parent = p;
        }
    }
    let n = r.u16("animation count").ok_or_else(|| r.error.clone())?;
    for _ in 0..n {
        let mut a = Animation::default();
        a.node = r.id("animation name").ok_or_else(|| r.error.clone())?;
        if at_least(f.version, 1.96) {
            r.u32("animation ID").ok_or_else(|| r.error.clone())?;
        }
        let frames = r
            .u16("animation frame count")
            .ok_or_else(|| r.error.clone())? as usize;
        a.period = f32::from_bits(r.u32("animation duration").ok_or_else(|| r.error.clone())?);
        a.kind = r.u8("animation type").ok_or_else(|| r.error.clone())?;
        let extra = r
            .u8("animation extra flag")
            .ok_or_else(|| r.error.clone())?;
        if !a.period.is_finite() || a.period < 0. {
            a.period = 0.;
        }
        if a.kind > 5 {
            a.kind = 0;
        }
        let stride = match a.kind {
            0 => 48,
            1 => 20,
            2 | 4 => 12,
            3 => 8,
            _ => 4,
        };
        let main = frames * stride;
        if main > r.remaining() {
            return Err(b("Invalid M3D animation payload length."));
        }
        if a.kind == 0 {
            for _ in 0..frames {
                let mut m = [0.; 12];
                for v in &mut m {
                    *v = r.f32("animation matrix").ok_or_else(|| r.error.clone())?;
                }
                a.matrices.push(m);
            }
        } else if a.kind == 5 {
            for _ in 0..frames {
                a.scalars
                    .push(r.f32("scalar animation").ok_or_else(|| r.error.clone())?);
            }
        } else {
            a.supported = false;
            r.skip(main, "animation payload")
                .ok_or_else(|| r.error.clone())?;
        }
        if extra != 0 {
            r.skip(frames * 4, "animation extra data")
                .ok_or_else(|| r.error.clone())?;
        }
        if a.kind == 0 || a.kind == 5 {
            a.supported = true;
        }
        f.animations.push(a);
    }
    if at_least(f.version, 1.96) {
        r.u32("database value").ok_or_else(|| r.error.clone())?;
    }
    if at_least(f.version, 1.6) {
        let n = r.u16("reference count").ok_or_else(|| r.error.clone())?;
        for _ in 0..n {
            let mut reference = Reference::default();
            reference.kind = r.u8("reference kind").ok_or_else(|| r.error.clone())? as i8;
            reference.node = r.id("reference node").ok_or_else(|| r.error.clone())?;
            reference.animation = r.id("reference animation").ok_or_else(|| r.error.clone())?;
            if at_least(f.version, 1.8) {
                reference.extra = r.u32("reference value").ok_or_else(|| r.error.clone())?;
            }
            f.references.push(reference);
        }
    }
    if r.remaining() != 0 {
        return Err(b(&format!(
            "M3D contains unrecognized trailing data at byte {}.",
            r.at
        )));
    }
    Ok(f)
}

#[derive(Default)]
// Counters written by the hierarchy clean-up; only `warnings` is reported.
#[allow(dead_code)]
struct Stats {
    source_version: f32,
    materials: usize,
    source_nodes: usize,
    output_nodes: usize,
    meshes: usize,
    groups: usize,
    triangles: usize,
    animations: usize,
    texture_references: usize,
    source_roots: usize,
    synthesized_root: bool,
    reordered_nodes: usize,
    disambiguated_names: usize,
    damage_emitters_adapted: usize,
    materials_end: usize,
    nodes_end: usize,
    animations_end: usize,
    references_end: usize,
    passthrough: bool,
    warnings: Vec<Bytes>,
    hierarchy: Vec<(Bytes, Bytes, Bytes)>,
    textures: Vec<TextureBinding>,
}
#[allow(dead_code)]
struct TextureBinding {
    mesh: Bytes,
    diffuse: Bytes,
    bump: Bytes,
    min: [f32; 3],
    max: [f32; 3],
    has_bounds: bool,
}
fn warning(s: &mut Stats, parts: &[&[u8]]) {
    s.warnings.push(join(parts));
}
fn normalize_texture(value: &[u8]) -> Bytes {
    let name = match value.iter().rposition(|c| *c == b'/' || *c == b'\\') {
        Some(i) => &value[i + 1..],
        None => value,
    };
    let lower = lower_ascii(name);
    for ext in [b".dds".as_slice(), b".tga", b".png", b".bmp"] {
        if lower.ends_with(ext) {
            return name[..name.len() - ext.len()].to_vec();
        }
    }
    name.to_vec()
}
fn sod_material(shader: &[u8]) -> &'static [u8] {
    let lower = lower_ascii(shader);
    let contains = |needle: &[u8]| lower.windows(needle.len()).any(|w| w == needle);
    if contains(b"alphathreshold") || (contains(b"alpha") && contains(b"threshold")) {
        b"alphathreshold"
    } else if contains(b"add") {
        b"additive"
    } else if contains(b"transluc") {
        b"translucent"
    } else if contains(b"wire") {
        b"wireframe"
    } else if contains(b"alpha") {
        b"alpha"
    } else {
        b"default"
    }
}
fn normalize_roots(source: &File, s: &mut Stats) -> File {
    let mut f = source.clone();
    let names: BTreeSet<Bytes> = f
        .nodes
        .iter()
        .filter(|n| !n.name.is_empty())
        .map(|n| n.name.clone())
        .collect();
    for node in &mut f.nodes {
        if !node.parent.is_empty() && (node.parent == node.name || !names.contains(&node.parent)) {
            warning(
                s,
                &[
                    b"Legacy M3D node '",
                    &node.name,
                    b"' references missing/self parent '",
                    &node.parent,
                    b"'; treating it as a top-level node.",
                ],
            );
            node.parent.clear();
        }
    }
    let roots: Vec<usize> = f
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent.is_empty())
        .map(|(i, _)| i)
        .collect();
    s.source_roots = roots.len();
    if roots.len() > 1 {
        let names: BTreeSet<Bytes> = f.nodes.iter().map(|n| n.name.clone()).collect();
        let mut root = b("a2fo_m3d_root");
        let mut suffix = 2;
        while names.contains(&root) {
            root = join(&[b"a2fo_m3d_root_", &num(suffix)]);
            suffix += 1;
        }
        for &i in &roots {
            f.nodes[i].parent = root.clone();
        }
        let n = Node {
            name: root.clone(),
            ..Node::default()
        };
        f.nodes.insert(0, n);
        s.synthesized_root = true;
        warning(
            s,
            &[
                b"Legacy M3D has ",
                &num(roots.len()),
                b" top-level nodes; inserted synthetic Storm3D root '",
                &root,
                b"' so geometry is not detached by Armada's single-root loader.",
            ],
        );
    } else if roots.is_empty() && !f.nodes.is_empty() {
        f.nodes[0].parent.clear();
        s.source_roots = 1;
        warning(
            s,
            &[
                b"Legacy M3D hierarchy contained no top-level node; promoted '",
                &f.nodes[0].name,
                b"' to the Storm3D root.",
            ],
        );
    }
    f
}

type Affine = [f64; 12];
fn compose(a: &Affine, b: &Affine) -> Affine {
    let mut out = [0.; 12];
    for col in 0..4 {
        for row in 0..3 {
            for k in 0..3 {
                out[col * 3 + row] += a[k * 3 + row] * b[col * 3 + k];
            }
            if col == 3 {
                out[col * 3 + row] += a[9 + row];
            }
        }
    }
    out
}
fn inverse(m: &Affine) -> Option<Affine> {
    let mut rows = [[0.; 6]; 3];
    for r in 0..3 {
        for c in 0..3 {
            rows[r][c] = m[c * 3 + r];
        }
        rows[r][r + 3] = 1.;
    }
    for c in 0..3 {
        let mut pivot = c;
        for r in c + 1..3 {
            if rows[r][c].abs() > rows[pivot][c].abs() {
                pivot = r;
            }
        }
        if rows[pivot][c].abs() < 1e-12 {
            return None;
        }
        rows.swap(c, pivot);
        let div = rows[c][c];
        for v in &mut rows[c] {
            *v /= div;
        }
        for r in 0..3 {
            if r == c {
                continue;
            }
            let factor = rows[r][c];
            for k in 0..6 {
                rows[r][k] -= factor * rows[c][k];
            }
        }
    }
    let mut out = [0.; 12];
    for r in 0..3 {
        for c in 0..3 {
            out[c * 3 + r] = rows[r][c + 3];
            out[9 + r] -= rows[r][c + 3] * m[9 + c];
        }
    }
    Some(out)
}
fn parent_index(node: &Node, by_name: &BTreeMap<Bytes, usize>) -> Option<usize> {
    by_name.get(&node.parent).copied()
}
fn world(i: usize, nodes: &[Node], by_name: &BTreeMap<Bytes, usize>) -> Option<Affine> {
    let mut result = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];
    let mut current = Some(i);
    let mut visited = 0;
    while let Some(index) = current {
        if visited >= nodes.len() {
            return None;
        }
        visited += 1;
        let mut local = [0.; 12];
        for (dst, src) in local.iter_mut().zip(&nodes[index].matrix) {
            *dst = *src as f64;
        }
        result = compose(&local, &result);
        current = parent_index(&nodes[index], by_name);
    }
    Some(result)
}
fn adapt_damage(f: &mut File, s: &mut Stats) {
    let mut by_name = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for (i, n) in f.nodes.iter().enumerate() {
        by_name.entry(n.name.clone()).or_insert(i);
        *counts.entry(authored(n)).or_insert(0usize) += 1;
    }
    for i in 0..f.nodes.len() {
        if f.nodes[i].kind != 12 {
            continue;
        }
        let Some(break_i) = parent_index(&f.nodes[i], &by_name) else {
            continue;
        };
        if authored(&f.nodes[break_i]) != b"break" {
            continue;
        }
        let Some(part_i) = parent_index(&f.nodes[break_i], &by_name) else {
            continue;
        };
        let part = authored(&f.nodes[part_i]);
        let selector: &[u8] = if part == b"l_engine" || part == b"r_engine" || part == b"engine" {
            b"engines"
        } else if part == b"weapons" {
            b"target"
        } else {
            continue;
        };
        let mut target = None;
        if counts.get(selector).copied() == Some(1) {
            for (j, n) in f.nodes.iter().enumerate() {
                if n.kind == 0
                    && authored(n) == selector
                    && let Some(parent) = parent_index(n, &by_name)
                    && authored(&f.nodes[parent]) == b"damage"
                {
                    target = Some(j);
                }
            }
        }
        let children = f.nodes.iter().any(|n| n.parent == f.nodes[i].name);
        let transform = target.and_then(|j| {
            if children || !f.animations.is_empty() || !f.references.is_empty() {
                return None;
            }
            let ew = world(i, &f.nodes, &by_name)?;
            let tw = world(j, &f.nodes, &by_name)?;
            Some((j, compose(&inverse(&tw)?, &ew)))
        });
        let Some((j, local)) = transform else {
            warning(
                s,
                &[
                    b"Damage emitter '",
                    &f.nodes[i].name,
                    b"' retained under break: requires a unique static damage/",
                    selector,
                    b" selector and invertible transform.",
                ],
            );
            continue;
        };
        if local
            .iter()
            .any(|v| !v.is_finite() || v.abs() > f32::MAX as f64)
        {
            continue;
        }
        for (dst, src) in f.nodes[i].matrix.iter_mut().zip(local) {
            *dst = src as f32;
        }
        f.nodes[i].parent = f.nodes[j].name.clone();
        s.damage_emitters_adapted += 1;
        warning(
            s,
            &[
                b"Mapped Legacy break emitter '",
                &f.nodes[i].name,
                b"' to native damage/",
                selector,
                b" with its model-space transform preserved.",
            ],
        );
    }
}
fn order_nodes(mut f: File, s: &mut Stats) -> File {
    if f.nodes.len() < 2 {
        return f;
    }
    let mut ordered = Vec::with_capacity(f.nodes.len());
    let mut emitted = vec![false; f.nodes.len()];
    let mut names = BTreeSet::new();
    let mut indices = Vec::new();
    let root = f
        .nodes
        .iter()
        .find(|n| n.parent.is_empty())
        .map(|n| n.name.clone())
        .unwrap_or_default();
    while ordered.len() < f.nodes.len() {
        let mut progress = false;
        for (i, n) in f.nodes.iter().enumerate() {
            if emitted[i] || (!n.parent.is_empty() && !names.contains(&n.parent)) {
                continue;
            }
            ordered.push(n.clone());
            indices.push(i);
            emitted[i] = true;
            if !n.name.is_empty() {
                names.insert(n.name.clone());
            }
            progress = true;
        }
        if progress {
            continue;
        }
        let Some(i) = emitted.iter().position(|e| !*e) else {
            break;
        };
        let mut n = f.nodes[i].clone();
        warning(
            s,
            &[
                b"Legacy M3D hierarchy cycle/order deadlock at node '",
                &n.name,
                b"' (parent '",
                &n.parent,
                b"'); reparented to root '",
                &root,
                b"' for Storm3D's one-pass loader.",
            ],
        );
        n.parent = root.clone();
        ordered.push(n);
        indices.push(i);
        emitted[i] = true;
        if !ordered.last().unwrap().name.is_empty() {
            names.insert(ordered.last().unwrap().name.clone());
        }
    }
    for (i, j) in indices.iter().enumerate() {
        if i != *j {
            s.reordered_nodes += 1;
        }
    }
    if s.reordered_nodes != 0 {
        warning(
            s,
            &[
                b"Reordered ",
                &num(s.reordered_nodes),
                b" Legacy node position(s) so every Storm3D parent is emitted before its children.",
            ],
        );
    }
    f.nodes = ordered;
    f
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn map(b: &[u8]) -> Option<String> {
    let n = normalize_texture(b);
    (!n.is_empty()).then(|| text(&n))
}

/// Load an M3D into a [`crate::model::Model`]. Version 1.93 reuses the existing
/// old-layout reader in memory. Newer layouts retain the A2FOM3D hierarchy
/// clean-up (root synthesis, unique names, damage emitters, parent-first order)
/// and diffuse, bump, specular, glow and detail references. No files are written.
pub fn load(input: &[u8]) -> Result<crate::model::Model, String> {
    use crate::model::{Channel, Group as MGroup, Keys, LightingMaterial, Maps, Mesh as MMesh, Model, Node as MNode, NodeKind, SourceFormat};
    use a2fmt::Matrix34;

    // Native Legacy accepts both magics before the same version dispatch. Its
    // 1.93 layout is the old SOD layout, not the newer M3D mesh/group layout.
    // Reuse that strict reader in memory only; never rewrite the user's file.
    if input.get(..8) == Some(b"Mad3D_SW")
        && input.get(10..14).is_some_and(|v| v == 1.93_f32.to_le_bytes())
    {
        let mut normalized = input.to_vec();
        normalized[..10].copy_from_slice(b"Storm3D_SW");
        let sod = a2fmt::sod::Sod::parse(&normalized).map_err(|e| format!("M3D 1.93: {e:?}"))?;
        let mut model = Model::from_sod(&sod);
        model.format = SourceFormat::M3d;
        for (node, source) in model.nodes.iter_mut().zip(&sod.nodes) {
            if let NodeKind::Light(light) = &node.kind
                && light.colour.into_iter().chain([light.falloff_start, light.falloff_range]).any(|v| !v.is_finite())
            {
                return Err("Non-finite M3D 1.93 light data.".into());
            }
            if source.node_type == 11 {
                node.kind = NodeKind::Lod;
            }
        }
        if model.nodes.iter().any(|node| matches!(node.kind, NodeKind::Light(_))) {
            model.warnings.push("Legacy light fields parsed; renderer light playback unavailable".into());
        }
        return Ok(model);
    }

    let format_version = |bits: u32| format!("{}", f32::from_bits(bits)).into_bytes();
    let source = parse(input, &format_version).map_err(|e| text(&e))?;
    let mut s = Stats::default();
    let mut f = normalize_roots(&source, &mut s);
    adapt_damage(&mut f, &mut s);
    let f = order_nodes(f, &mut s);

    let mut warnings: Vec<String> = s.warnings.iter().map(|w| text(w)).collect();
    let mut by_name: BTreeMap<Bytes, usize> = BTreeMap::new();
    for (i, n) in f.nodes.iter().enumerate() {
        by_name.entry(lower_ascii(&n.name)).or_insert(i);
    }
    let mut nodes: Vec<MNode> = f
        .nodes
        .iter()
        .map(|n| {
            let kind = match n.kind {
                1 => NodeKind::Mesh(MMesh {
                    positions: n.mesh.vertices.iter().map(|v| v.map(finite)).collect(),
                    uvs: n.mesh.uvs.iter().map(|v| v.map(finite)).collect(),
                    two_sided: n.mesh.cull == 0,
                    groups: n
                        .mesh
                        .groups
                        .iter()
                        .map(|g| MGroup {
                            material: text(&g.material),
                            blend: text(sod_material(&g.shader)),
                            maps: {
                                // Shipped STL models put their `*_glow` texture in the
                                // slot after specular (read here as `detail`); take a
                                // glow texture as the glow map whichever slot it is in.
                                let is_glow = |t: &Option<String>| t.as_ref().is_some_and(|n| n.to_ascii_lowercase().ends_with("_glow"));
                                let (mut glow, mut detail) = (map(&g.glow), map(&g.detail));
                                if glow.is_none() && is_glow(&detail) {
                                    glow = detail.take();
                                }
                                Maps {
                                    diffuse: map(&g.diffuse),
                                    height: None,
                                    bump: map(&g.bump),
                                    specular: map(&g.specular),
                                    glow,
                                    detail,
                                }
                            },
                            // M3D winds the other way from SOD (A2FOM3D writes [0, 2, 1]).
                            faces: g
                                .faces
                                .iter()
                                .filter(|fc| {
                                    fc.positions.iter().all(|&p| (p as usize) < n.mesh.vertices.len())
                                        && fc.uvs.iter().all(|&u| (u as usize) < n.mesh.uvs.len())
                                })
                                .map(|fc| [0, 2, 1].map(|c| (fc.positions[c], fc.uvs[c])))
                                .collect(),
                        })
                        .collect(),
                }),
                3 => NodeKind::Sprite,
                12 => NodeKind::Emitter(text(if n.payload.is_empty() { &n.name } else { &n.payload })),
                0 => NodeKind::Null,
                2 => {
                    warnings.push(format!("Legacy light '{}' parsed; renderer light playback unavailable", text(&n.name)));
                    NodeKind::Light(n.light.clone().expect("type-2 payload is checked while parsing"))
                }
                11 => NodeKind::Lod,
                k => {
                    warnings.push(format!("node '{}' of type {k} kept as a plain transform", text(&n.name)));
                    NodeKind::Null
                }
            };
            MNode {
                name: text(&n.name),
                parent: if n.parent.is_empty() { None } else { by_name.get(&lower_ascii(&n.parent)).copied() },
                children: Vec::new(),
                transform: Matrix34 { m: n.matrix },
                kind,
            }
        })
        .collect();
    for i in 0..nodes.len() {
        if let Some(p) = nodes[i].parent {
            if p == i {
                nodes[i].parent = None;
            } else {
                nodes[p].children.push(i);
            }
        }
    }
    let channels = f
        .animations
        .iter()
        .filter(|a| a.supported)
        .filter_map(|a| {
            let frames = if a.kind == 5 { a.scalars.len() } else { a.matrices.len() };
            let period = if a.period.is_finite() && a.period > 0. { a.period } else { frames as f32 / 30. };
            let keys = if a.kind == 5 {
                Keys::Scale(a.scalars.iter().map(|v| finite(*v)).collect())
            } else {
                Keys::Matrix(a.matrices.iter().map(|m| Matrix34 { m: *m }).collect())
            };
            (frames > 0).then(|| Channel { node: text(&a.node), period, keys })
        })
        .collect();
    let skipped = f.animations.iter().filter(|a| !a.supported).count();
    if skipped > 0 {
        warnings.push(format!("{skipped} Legacy-only animation channel(s) skipped"));
    }
    Ok(Model {
        format: SourceFormat::M3d,
        version: f.version,
        materials: f
            .materials
            .iter()
            .map(|m| LightingMaterial {
                name: text(&m.name),
                ambient: [0.0; 3],
                diffuse: m.diffuse,
                specular: m.specular,
                power: finite(m.shininess),
                model: 2,
                self_illuminating: m.flags2 != 0,
            })
            .collect(),
        nodes,
        channels,
        flipbooks: f
            .references
            .iter()
            .filter(|r| r.kind == 4)
            .map(|r| (text(&r.node), text(&r.animation), finite(f32::from_bits(r.extra))))
            .collect(),
        warnings,
        pbr: None,
    })
}

#[cfg(test)]
#[path = "m3d/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "m3d/install_tests.rs"]
mod install_tests;
