//! Strategic-view silhouettes: a model seen from straight above, as a coverage mask.
//!
//! The same look as the strategic sprite sets baked for FO4 / STA1 / STA2 Classic
//! (`<model>_sv`): orthographic top view, model `+X` image-right, model `+Z`
//! (the front) image-up, centred, the larger extent filling the sprite less its
//! padding. Only what a healthy craft shows counts: the most detailed LOD, no
//! damage-state or Borg nodes, no additive glows, shields or sprites.

use crate::model::{Model, NodeKind};

/// A `size` x `size` coverage mask (0..255), row 0 at the top.
#[derive(Debug, Clone)]
pub struct Silhouette {
    pub size: u32,
    pub alpha: Vec<u8>,
}

/// Bake `model`'s top view; `None` when it has no solid geometry.
pub fn bake(model: &Model, size: u32, pad: u32) -> Option<Silhouette> {
    let mut hidden = vec![false; model.nodes.len()];
    for i in model.coarse_lods().into_iter().chain(model.damage_nodes()).chain(model.borg_node()) {
        hidden[i] = true;
    }
    let shown = |mut i: usize| loop {
        if hidden[i] {
            return false;
        }
        match model.nodes[i].parent {
            Some(p) => i = p,
            None => return true,
        }
    };
    // Every solid triangle, top-down: (x, z) in model space.
    let mut tris: Vec<[[f32; 2]; 3]> = Vec::new();
    for (i, n) in model.nodes.iter().enumerate() {
        let NodeKind::Mesh(mesh) = &n.kind else { continue };
        if !shown(i) {
            continue;
        }
        let m = model.model_transform(i);
        let flat: Vec<[f32; 2]> = mesh.positions.iter().map(|&p| { let q = m.transform_point(p); [q[0], q[2]] }).collect();
        for g in &mesh.groups {
            let b = g.blend.to_ascii_lowercase();
            if matches!(b.as_str(), "additive" | "wormhole" | "atmosphere") {
                continue;
            }
            for f in &g.faces {
                let c = [f[0].0, f[1].0, f[2].0].map(|v| flat.get(v as usize).copied().unwrap_or([0.0, 0.0]));
                tris.push(c);
            }
        }
    }
    if tris.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for t in &tris {
        for p in t {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    if extent <= 0.0 {
        return None;
    }
    let inner = size.saturating_sub(2 * pad).max(1) as f32;
    let scale = inner / extent;
    let half = size as f32 * 0.5;
    let (cx, cz) = ((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
    // Into pixel space: x right, z up.
    let to_px = |p: [f32; 2]| [(p[0] - cx) * scale + half, half - (p[1] - cz) * scale];
    // 4x4 samples per pixel.
    const SS: u32 = 4;
    let n = size * SS;
    let mut hits = vec![0u8; (n * n) as usize];
    for t in &tris {
        let [a, b, c] = t.map(|p| to_px(p).map(|v| v * SS as f32));
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as u32;
        let x1 = (a[0].max(b[0]).max(c[0]).ceil() as u32).min(n);
        let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as u32;
        let y1 = (a[1].max(b[1]).max(c[1]).ceil() as u32).min(n);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let e = |u: [f32; 2], v: [f32; 2]| ((v[0] - u[0]) * (p[1] - u[1]) - (v[1] - u[1]) * (p[0] - u[0])) * area.signum();
                if e(a, b) >= 0.0 && e(b, c) >= 0.0 && e(c, a) >= 0.0 {
                    hits[(y * n + x) as usize] = 1;
                }
            }
        }
    }
    let mut alpha = vec![0u8; (size * size) as usize];
    for y in 0..size {
        for x in 0..size {
            let mut c = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    c += hits[((y * SS + sy) * n + x * SS + sx) as usize] as u32;
                }
            }
            alpha[(y * size + x) as usize] = (c * 255 / (SS * SS)) as u8;
        }
    }
    Some(Silhouette { size, alpha })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2fmt::Matrix34;
    use crate::model::{Group, Maps, Mesh, Node};

    fn node(name: &str, parent: Option<usize>, children: Vec<usize>, kind: NodeKind) -> Node {
        Node { name: name.into(), parent, children, transform: Matrix34::IDENTITY, kind }
    }

    fn quad(x0: f32, z0: f32, x1: f32, z1: f32, blend: &str) -> NodeKind {
        NodeKind::Mesh(Mesh {
            positions: vec![[x0, 0.0, z0], [x1, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1]],
            uvs: vec![[0.0, 0.0]],
            groups: vec![Group {
                material: String::new(),
                blend: blend.into(),
                maps: Maps::default(),
                faces: vec![[(0, 0), (1, 0), (2, 0)], [(0, 0), (2, 0), (3, 0)]],
            }],
            two_sided: false,
        })
    }

    fn model(nodes: Vec<Node>) -> Model {
        Model { format: crate::model::SourceFormat::Sod, version: 1.93, materials: vec![], nodes, channels: vec![], flipbooks: vec![], warnings: vec![], pbr: None }
    }

    fn at(s: &Silhouette, x: u32, y: u32) -> u8 {
        s.alpha[(y * s.size + x) as usize]
    }

    #[test]
    fn front_is_up_and_right_is_right() {
        // A wide hull with a bow block toward +Z and a block on +X.
        let m = model(vec![
            node("root", None, vec![1, 2, 3], NodeKind::Null),
            node("hull", Some(0), vec![], quad(-10.0, -10.0, 10.0, 0.0, "default")),
            node("bow", Some(0), vec![], quad(-2.0, 0.0, 2.0, 10.0, "default")),
            node("glow", Some(0), vec![], quad(-10.0, 0.0, -5.0, 10.0, "additive")),
        ]);
        let s = bake(&m, 32, 2).unwrap();
        // The bow (+Z) is in the top half, centred; the hull fills the bottom half.
        assert!(at(&s, 16, 6) > 200, "bow at top");
        assert!(at(&s, 16, 24) > 200 && at(&s, 4, 24) > 200 && at(&s, 27, 24) > 200, "hull at bottom");
        // Beside the bow is clear, including where the additive glow is.
        assert_eq!(at(&s, 26, 6), 0);
        assert_eq!(at(&s, 5, 6), 0, "additive parts don't count");
        // The padding stays clear.
        assert_eq!(at(&s, 0, 16), 0);
    }

    #[test]
    fn damage_and_borg_nodes_do_not_count() {
        let m = model(vec![
            node("root", None, vec![1, 2, 3], NodeKind::Null),
            node("hull", Some(0), vec![], quad(-1.0, -1.0, 1.0, 1.0, "default")),
            node("damage", Some(0), vec![], quad(-50.0, -50.0, 50.0, 50.0, "default")),
            node("borg", Some(0), vec![], quad(-50.0, -50.0, 50.0, 50.0, "default")),
        ]);
        let s = bake(&m, 16, 1).unwrap();
        // Only the 2x2 hull: it fills the sprite, so the bounds ignored the big nodes.
        assert!(at(&s, 8, 8) > 200 && at(&s, 2, 2) > 200);
    }

    #[test]
    fn nothing_solid_gives_none() {
        let m = model(vec![node("glow", None, vec![], quad(-1.0, -1.0, 1.0, 1.0, "additive"))]);
        assert!(bake(&m, 16, 1).is_none());
    }
}
